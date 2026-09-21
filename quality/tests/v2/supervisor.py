#!/usr/bin/env python3
"""CLI lifecycle, runtime binding and immutable configuration acceptance."""
import contextlib
import json
import os
import signal
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from support import App, BIN, EXE, eventually


def file_plugin(path, autostart=False, identity='source', alias='source-log', mode='static'):
    return {'id':identity,'role':'input','bin':'input-file','autostart':autostart,
            'streams':[{'id':alias,'description':'fixture bytes'}],
            'config':{'path':str(path),'mode':mode,'from_start':True,'chunk_bytes':4}}


class OwnedProcess:
    """Observe a fixture process without confusing PID reuse with survival.

    Windows uses a stable kernel process handle. Unix checks the captured start
    time and full command before any signal, and treats exited zombies as dead.
    """
    def __init__(self, pid):
        self.pid = pid
        self.handle = None
        if os.name == 'nt':
            import ctypes
            from ctypes import wintypes
            self.kernel = ctypes.WinDLL('kernel32', use_last_error=True)
            self.kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
            self.kernel.OpenProcess.restype = wintypes.HANDLE
            self.kernel.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
            self.kernel.WaitForSingleObject.restype = wintypes.DWORD
            self.kernel.TerminateProcess.argtypes = [wintypes.HANDLE, wintypes.UINT]
            self.kernel.TerminateProcess.restype = wintypes.BOOL
            self.kernel.CloseHandle.argtypes = [wintypes.HANDLE]
            self.kernel.CloseHandle.restype = wintypes.BOOL
            # SYNCHRONIZE | PROCESS_TERMINATE: only these owned fixtures.
            self.handle = self.kernel.OpenProcess(0x00100000 | 0x0001, False, pid)
            if not self.handle:
                raise ctypes.WinError(ctypes.get_last_error())
        else:
            info = self._unix_info()
            assert info is not None and not info[0].startswith('Z'), ('fixture is not alive', pid, info)
            self.identity = info[1]

    def _unix_info(self):
        result = subprocess.run(['ps','-ww','-o','stat=','-o','lstart=','-o','command=','-p',str(self.pid)],
                                capture_output=True,text=True,timeout=3)
        text = result.stdout.strip()
        return tuple(text.split(None,1)) if text else None

    def alive(self):
        if self.handle:
            result = self.kernel.WaitForSingleObject(self.handle, 0)
            if result not in (0, 0x00000102):
                raise AssertionError(('WaitForSingleObject failed', self.pid, result))
            return result == 0x00000102
        info = self._unix_info()
        return bool(info and info[1] == self.identity and not info[0].startswith('Z'))

    def kill(self):
        if not self.alive():
            return
        if self.handle:
            if not self.kernel.TerminateProcess(self.handle, 97):
                import ctypes
                raise ctypes.WinError(ctypes.get_last_error())
        else:
            with contextlib.suppress(ProcessLookupError):
                os.kill(self.pid, signal.SIGKILL)

    def close(self):
        if self.handle:
            self.kernel.CloseHandle(self.handle)
            self.handle = None


class SupervisorTests(unittest.TestCase):
    def test_starts_empty_and_stays_running_until_manual_stop(self):
        with App({'plugins':[]}) as app:
            self.assertEqual(app.json('streams'),[])
            time.sleep(.15)
            self.assertEqual(app.json('streams'),[])
            self.assertIn('supervisor',app.json('status'))
            self.assertFalse(app.cli('config','set','x','--json','{}',ok=False).returncode==0)
            self.assertFalse(app.cli('session','list','x',ok=False).returncode==0)

    def test_late_start_returns_stream_id_snapshot_survives_file_edit_and_restart(self):
        with tempfile.TemporaryDirectory() as td:
            source = Path(td)/'source.log'
            replacement = Path(td)/'replacement.log'
            source.write_bytes(b'original\n')
            replacement.write_bytes(b'wrong\n')
            config = {'plugins':[file_plugin(source)]}
            with App(config) as app:
                # Changed disk file is valid but must not be consulted by late children.
                config['plugins'][0]['config']['path'] = str(replacement)
                app.config.write_text(json.dumps(config))
                started = app.json('plugin','start','source')
                self.assertTrue(started['streams'])
                stream = started['streams'][0]['id']
                data = eventually(lambda: app.json('read',stream)['records'])
                eventually(lambda: next(p for p in app.json('status')['plugin_processes'] if p['id']=='source')['state']=='stopped')
                data = app.json('read',stream)['records']
                self.assertEqual(bytes(x for r in data for x in r['payload']),b'original\n')
                self.assertEqual(app.json('stream',stream)['description'],'fixture bytes')
                self.assertEqual(app.json('config')['config']['plugins'][0]['config']['path'],str(source))
                app.json('plugin','restart','source')
                eventually(lambda: len(app.json('read',stream)['records'])>=6)
                self.assertEqual(bytes(x for r in app.json('read',stream)['records'] for x in r['payload']),b'original\noriginal\n')

    def test_runtime_output_binding_uses_real_stream_id(self):
        with tempfile.TemporaryDirectory() as td:
            source = Path(td)/'source.log'
            source.write_bytes(b'bound-by-UUID\n')
            config = {'plugins':[file_plugin(source),{'id':'screen','role':'output','bin':'output-raw','autostart':False,'config':{}}]}
            with App(config) as app:
                started = app.json('plugin','start','source')
                stream = started['streams'][0]['id']
                app.json('plugin','start','screen','--stream',stream)
                stdout = app.path/'state.json.stdout.log'
                eventually(lambda: stdout.exists() and b'bound-by-UUID\n' in stdout.read_bytes())
                result = app.json('plugin','stop','screen')
                self.assertFalse(result['forced'])
                self.assertTrue(result['success'])
                self.assertTrue(app.json('read',stream)['records'])

    def test_start_failure_cleans_its_state(self):
        with tempfile.TemporaryDirectory():
            app = App({'plugins':[file_plugin('/definitely/not/a/log/file',autostart=True)]})
            try:
                result = app.cli('start','--config',app.config,ok=False)
                self.assertNotEqual(result.returncode,0)
                eventually(lambda:not app.state.exists())
            finally:
                app.temp.cleanup()

    def test_core_crash_reaps_spawned_source_and_descendant(self):
        # TCP detects EOF; UDP has no such notification, so the supervisor's
        # direct cooperative termination fallback is exercised separately.
        for transport in ('tcp', 'udp'):
            with self.subTest(transport=transport), tempfile.TemporaryDirectory(prefix='log-print-crash-') as td:
                directory = Path(td)
                marker = directory/'owned-processes.json'
                source = directory/'source.py'
                source.write_text(
                    "import json,os,pathlib,subprocess,sys,time\n"
                    "child=subprocess.Popen([sys.executable,'-c','import time;time.sleep(120)',sys.argv[1]])\n"
                    "pathlib.Path(sys.argv[1]).write_text(json.dumps([os.getpid(),child.pid]))\n"
                    "os.write(1,b'owned process tree ready\\n')\n"
                    "time.sleep(120)\n")
                config = {'core':{'transport':transport},'plugins':[{
                    'id':'source','role':'input','bin':'input-program',
                    'streams':[{'id':'crash-source'}],
                    'config':{'command':sys.executable,'args':['-u',str(source),str(marker)]},
                }]}
                app = App(config)
                observed = []
                try:
                    app.start()
                    def source_pids():
                        try:
                            return json.loads(marker.read_text())
                        except (OSError, ValueError):
                            return None
                    pids = eventually(source_pids)
                    snapshot = app.json('status')
                    state = json.loads(app.state.read_text())
                    wrapper_pid = next(p['pid'] for p in snapshot['plugin_processes'] if p['id']=='source')
                    for pid in [state['core_pid'], state['pid'], wrapper_pid, *pids]:
                        observed.append(OwnedProcess(pid))
                    self.assertTrue(all(process.alive() for process in observed))
                    # Fault injection kills only this isolated test's Core.
                    observed[0].kill()
                    eventually(lambda:not app.state.exists(), timeout=25)
                    eventually(lambda:all(not process.alive() for process in observed), timeout=8)
                    self.assertFalse(any(process.alive() for process in observed),
                                     'supervisor, wrapper, source, or descendant survived Core crash')
                finally:
                    # On assertion failure, clean only handles/identities that
                    # were proven to belong to this fixture before the crash.
                    if app.state.exists():
                        with contextlib.suppress(Exception):
                            app.cli('stop',ok=False,timeout=20)
                    for process in reversed(observed):
                        with contextlib.suppress(Exception):
                            process.kill()
                        process.close()
                    app.temp.cleanup()

    def test_old_storage_config_and_duplicate_identity_rejected(self):
        invalid = [
            {'core':{'save':{'enabled':True}}},
            {'plugins':[{'id':'bad','bin':'input-file'}]},
            {'plugins':[{'id':'x','role':'input','bin':'unused'},{'id':'x','role':'input','bin':'unused'}]},
        ]
        for config in invalid:
            with self.subTest(config=config):
                app = App(config)
                try:
                    result = app.cli('start','--config',app.config,ok=False)
                    self.assertNotEqual(result.returncode,0)
                    eventually(lambda:not app.state.exists())
                finally:
                    app.temp.cleanup()

    def test_second_start_does_not_stop_or_overwrite_existing_instance(self):
        with App({}) as app:
            before = app.state.read_bytes()
            result = app.cli('start','--config',app.config,ok=False)
            self.assertNotEqual(result.returncode,0)
            self.assertEqual(app.state.read_bytes(),before)
            self.assertIn('supervisor',app.json('status'))

    def test_udp_configuration_and_actual_file_input(self):
        with tempfile.TemporaryDirectory() as td:
            source = Path(td)/'source.log'
            source.write_bytes(b'udp')
            with App({'core':{'transport':'udp'},'plugins':[file_plugin(source,True)]}) as app:
                stream = app.json('streams')[0]['id']
                records = eventually(lambda:app.json('read',stream)['records'])
                self.assertEqual(bytes(x for r in records for x in r['payload']),b'udp')
                self.assertEqual(app.json('config')['config']['core']['transport'],'udp')


if __name__=='__main__':
    unittest.main(verbosity=2)
