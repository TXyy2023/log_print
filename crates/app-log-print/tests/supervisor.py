#!/usr/bin/env python3
"""Real supervisor/Core/Rust-plugin lifecycle checks. Build selected packages first.
Run: python3 crates/app-log-print/tests/supervisor.py [--binary /path/to/log-print]
Uses only processes and files created by this script; POSIX PPID/kill cases skip on Windows.
"""
import argparse
import json
import os
from pathlib import Path
import re
import signal
import socket
import stat
import sys
import subprocess
import tempfile
import threading
import time

ROOT = Path(__file__).resolve().parents[3]
parser = argparse.ArgumentParser()
parser.add_argument('--binary', type=Path, default=ROOT / 'target/debug' / ('log-print.exe' if os.name == 'nt' else 'log-print'))
parser.add_argument('--artifacts', type=Path, help='Retain per-CLI stdout/stderr files for diagnosis')
args = parser.parse_args()
BINARY = args.binary.resolve()
if os.name == 'nt':
    import ctypes
    from ctypes import wintypes
    kernel32 = ctypes.WinDLL('kernel32', use_last_error=True)
    kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel32.OpenProcess.restype = wintypes.HANDLE
    kernel32.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
    kernel32.WaitForSingleObject.restype = wintypes.DWORD
    kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
    kernel32.CloseHandle.restype = wintypes.BOOL


def alive(pid):
    if os.name == 'nt':
        # A zero-time wait observes the process; it neither signals nor kills it.
        handle = kernel32.OpenProcess(0x00100000, False, pid)  # SYNCHRONIZE only
        if not handle:
            if ctypes.get_last_error() == 87:  # ERROR_INVALID_PARAMETER: PID no longer exists
                return False
            raise ctypes.WinError(ctypes.get_last_error())
        try:
            result = kernel32.WaitForSingleObject(handle, 0)
            if result == 0:  # WAIT_OBJECT_0: process exited
                return False
            if result == 258:  # WAIT_TIMEOUT: process still running
                return True
            raise ctypes.WinError(ctypes.get_last_error())
        finally:
            kernel32.CloseHandle(handle)
    result = subprocess.run(['ps', '-o', 'stat=', '-p', str(pid)], capture_output=True, text=True)
    return result.returncode == 0 and result.stdout.strip() and not result.stdout.lstrip().startswith('Z')


def wait(predicate, timeout=12):
    until = time.monotonic() + timeout
    while time.monotonic() < until:
        if predicate():
            return
        time.sleep(.04)
    raise AssertionError('condition timed out')


with tempfile.TemporaryDirectory(prefix='log-print-supervisor-') as tmp:
    base = Path(tmp)
    cli_logs = args.artifacts.resolve() if args.artifacts else base / 'cli-logs'
    cli_logs.mkdir(parents=True, exist_ok=True)
    cli_number = 0
    state, config, source, output = [base / name for name in ('state.json', 'config.json', 'source.bin', 'output.bin')]
    source.write_bytes(b'')
    manifest = {'plugins': [
        {'id': 'file', 'bin': 'input-file', 'streams': [{'id': 'logs'}], 'config': {'path': str(source), 'stream': 'logs', 'poll_ms': 5}},
        {'id': 'raw', 'bin': 'output-raw', 'reads': ['logs'], 'config': {'streams': ['logs'], 'path': str(output), 'append': True}},
    ]}
    config.write_text(json.dumps(manifest))

    def pipe_result(command, stdout_path, stderr_path):
        process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, bufsize=0)
        captured = [bytearray(), bytearray()]
        eof = [threading.Event(), threading.Event()]
        read_errors = []
        def drain(index, stream):
            try:
                while chunk := os.read(stream.fileno(), 65536):
                    captured[index].extend(chunk)
            except OSError as error:
                read_errors.append(str(error))
            finally:
                stream.close()
                eof[index].set()
        for index, stream in enumerate((process.stdout, process.stderr)):
            threading.Thread(target=drain, args=(index, stream), daemon=True).start()
        deadline = time.monotonic() + 35
        try:
            code = process.wait(timeout=35)
            for event in eof:
                if not event.wait(max(0, deadline-time.monotonic())):
                    raise subprocess.TimeoutExpired(command, 35)
            assert not read_errors, read_errors
            return subprocess.CompletedProcess(command, code, bytes(captured[0]), bytes(captured[1]))
        except BaseException:
            print(f'PIPE capture failed: cli_exit={process.poll()} stdout_eof={eof[0].is_set()} '
                  f'stderr_eof={eof[1].is_set()}; stopping owned instance using file capture', flush=True)
            try:
                if state.exists():
                    cli('stop')
            except Exception as error:
                print(f'PIPE failure cleanup: {error}', flush=True)
            if process.poll() is None:
                process.kill()
                process.wait(timeout=5)
            cleanup_deadline = time.monotonic() + 5
            for event in eof:
                event.wait(max(0, cleanup_deadline-time.monotonic()))
            raise
        finally:
            stdout_path.write_bytes(bytes(captured[0]))
            stderr_path.write_bytes(bytes(captured[1]))

    def cli(*words, success=True, pipe=False):
        global cli_number
        cli_number += 1
        number = cli_number
        safe_words = list(words)
        for i, word in enumerate(safe_words[:-1]):
            if word in ('--json', '--token'):
                safe_words[i + 1] = '<redacted>'
        label = json.dumps(safe_words, ensure_ascii=True)
        before = time.monotonic()
        stdout_path = cli_logs / f'cli-{number:03}.stdout.log'
        stderr_path = cli_logs / f'cli-{number:03}.stderr.log'
        mode = 'PIPE' if pipe else 'file'
        print(f'CLI {number:03} begin mode={mode} {label}', flush=True)
        command = [str(BINARY), '--state', str(state), *words]
        if pipe:
            result = pipe_result(command, stdout_path, stderr_path)
        else:
            # Regular files remain available even if descendants inherited a handle.
            with stdout_path.open('wb') as stdout, stderr_path.open('wb') as stderr:
                process = subprocess.Popen(command, stdout=stdout, stderr=stderr)
                try:
                    code = process.wait(timeout=35)
                except (subprocess.TimeoutExpired, KeyboardInterrupt):
                    if process.poll() is None:
                        process.kill()
                    process.wait(timeout=5)
                    print(f'CLI {number:03} interrupted/timeout after {time.monotonic()-before:.3f}s; '
                          f'stdout={stdout_path.name} stderr={stderr_path.name}', flush=True)
                    raise
            result = subprocess.CompletedProcess(command, code, stdout_path.read_bytes(), stderr_path.read_bytes())
        print(f'CLI {number:03} end mode={mode} exit={result.returncode} seconds={time.monotonic()-before:.3f} {label}', flush=True)
        if success:
            assert result.returncode == 0, result.stderr.decode(errors='replace')
        else:
            assert result.returncode != 0, result.stdout
        return result

    def value(*words, pipe=False):
        return json.loads(cli(*words, pipe=pipe).stdout)

    owned = []
    try:
        started = value('start', '--config', str(config), pipe=True)
        status = value('status', pipe=True)
        owned = [started['pid'], started['core_pid']] + [p['pid'] for p in status['plugin_processes']]
        assert len(set(owned)) == 4, owned
        assert all(alive(pid) for pid in owned), 'background instance must remain healthy after PIPE EOF'
        assert all(p['connected'] for p in status['plugins'])
        if os.name != 'nt':
            for pid in owned[1:]:
                assert int(subprocess.check_output(['ps', '-o', 'ppid=', '-p', str(pid)])) == started['pid']
            assert stat.S_IMODE(state.stat().st_mode) == 0o600
            saved = json.loads(state.read_text())
            runtime = Path(saved['runtime_directory'])
            assert stat.S_IMODE(runtime.stat().st_mode) == 0o700
            assert stat.S_IMODE((runtime / 'config.json').stat().st_mode) == 0o600
        saved = json.loads(state.read_text())
        with socket.create_connection(tuple([saved['address'].rsplit(':', 1)[0], int(saved['address'].rsplit(':', 1)[1])]), timeout=3) as sock:
            sock.sendall((json.dumps({'protocol': 'log-print/1', 'plugin': '__manager__', 'token': 'incorrect', 'events': False}) + '\n').encode())
            rejected = json.loads(sock.makefile('rb').readline())
            assert rejected['error']['code'] == 'handshake_rejected'
        payload = b'no-newline\x00\xff\r' + '跨块'.encode()
        with source.open('ab') as stream:
            stream.write(payload)
            stream.flush()
        wait(lambda: output.exists() and output.read_bytes() == payload)
        assert cli('read', 'logs', '--raw', pipe=True).stdout == payload
        cursor = next(s['head'] for s in value('streams') if s['id'] == 'logs') + 1
        delayed = b'delayed\x00\xff'
        def append_delayed():
            with source.open('ab') as stream:
                stream.write(delayed)
                stream.flush()
        timer = threading.Timer(.2, append_delayed)
        before = time.monotonic()
        timer.start()
        try:
            page = value('read', 'logs', '--from', str(cursor), '--wait-ms', '2000')
        finally:
            timer.join()
        assert b''.join(bytes(record['payload']) for record in page['records']) == delayed
        assert .15 <= time.monotonic() - before < 2.5
        payload += delayed
        wait(lambda: output.read_bytes() == payload)
        cursor = next(s['head'] for s in value('streams') if s['id'] == 'logs') + 1
        before = time.monotonic()
        empty = value('read', 'logs', '--from', str(cursor), '--wait-ms', '120')
        assert empty['records'] == []
        assert .1 <= time.monotonic() - before < 2
        cli('read', 'logs', '--wait-ms', '60001', success=False)
        print('PASS bounded read wait observes delayed bytes and returns an honest empty timeout')
        changed = value('config', 'set', 'file', '--json', '{"poll_ms":7}')
        assert changed['effective']['poll_ms'] == 7
        assert value('config', '--plugin', 'file')['runtime']['effective']['poll_ms'] == 7
        cli('config', 'set', 'file', '--json', '{"poll_ms":0}', success=False)
        assert value('config', '--plugin', 'file')['runtime']['effective']['poll_ms'] == 7
        stopped = value('plugin', 'stop', 'raw')
        assert stopped['forced'] is False, stopped
        old_pid = next(p['pid'] for p in status['plugin_processes'] if p['id'] == 'raw')
        wait(lambda: not alive(old_pid))
        new = value('plugin', 'start', 'raw')
        owned.append(new['pid'])
        assert new['pid'] != old_pid
        wait(lambda: output.read_bytes() == payload * 2)
        restarted = value('plugin', 'restart', 'raw')
        owned.append(restarted['started']['pid'])
        assert restarted['stopped']['forced'] is False
        cli('stop', pipe=True)
        assert not state.exists()
        wait(lambda: all(not alive(pid) for pid in owned))
        assert not Path(saved['runtime_directory']).exists()
        print('PASS PIPE capture start/status/read/stop reaches both EOFs while preserving background lifecycle')
        print('PASS real bytes, sibling PPIDs, permissions, auth, runtime config, plugin restart, graceful stop')

        # Failure after one plugin was created must also reap the Core and remove state.
        broken = {'plugins': [manifest['plugins'][0] | {'id': 'a_file'}, {'id': 'z_missing', 'bin': str(base / 'absent-plugin'), 'config': {}}]}
        config.write_text(json.dumps(broken))
        failed = cli('run', '--config', str(config), success=False)
        assert not state.exists()
        failed_pids = [int(p) for p in re.findall(rb'pid=Some\((\d+)\)', failed.stderr)]
        assert len(failed_pids) >= 2, failed.stderr.decode(errors='replace')
        wait(lambda: all(not alive(pid) for pid in failed_pids))
        print('PASS partial startup cleanup')

        if os.name != 'nt':
            # Interrupt during plugin handshake, before the final ready state exists.
            waiting = {'plugins': [{'id': 'stall', 'bin': sys.executable, 'args': ['-c', 'import time; time.sleep(30)'], 'config': {}}]}
            config.write_text(json.dumps(waiting))
            startup_log = base / 'startup-interrupt.log'
            with startup_log.open('wb') as log:
                process = subprocess.Popen([str(BINARY), '--state', str(state), 'run', '--config', str(config)], stdout=log, stderr=log)
                try:
                    wait(lambda: 'spawned plugin stall' in startup_log.read_text())
                    process.send_signal(signal.SIGINT)
                    assert process.wait(timeout=12) != 0
                finally:
                    if process.poll() is None:
                        process.send_signal(signal.SIGTERM)
                        process.wait(timeout=12)
            assert not state.exists()
            interrupted_pids = [int(p) for p in re.findall(r'pid=Some\((\d+)\)', startup_log.read_text())]
            assert len(interrupted_pids) == 2
            wait(lambda: all(not alive(pid) for pid in interrupted_pids))
            print('PASS startup interruption cleans up owned processes')
            config.write_text(json.dumps(manifest))
            restarted = value('start', '--config', str(config))
            status = value('status')
            owned = [restarted['pid'], restarted['core_pid']] + [p['pid'] for p in status['plugin_processes']]
            os.kill(restarted['core_pid'], signal.SIGTERM)
            wait(lambda: not state.exists())
            wait(lambda: all(not alive(pid) for pid in owned))
            print('PASS Core death cleans up owned plugins')
        else:
            print('SKIP POSIX PPID/Core-death injection on Windows; other lifecycle checks ran')
    finally:
        if state.exists():
            try:
                cli('stop')
            except Exception as error:
                print(f'cleanup failed: {error}; owned PIDs: {owned}', flush=True)
                raise
