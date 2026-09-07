#!/usr/bin/env python3
"""Real child-process UI contract checks. Browser/visible-terminal review is separate.

python3 plugins/outputs/output-webui/tests/verify_ui.py --pty
--serve keeps only this freshly-created instance live for browser review until Ctrl-C.
"""
import argparse
import json
import math
import os
from pathlib import Path
import re
import select
import struct
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request

ROOT = Path(__file__).resolve().parents[4]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--skip-build', action='store_true')
    parser.add_argument('--pty', action='store_true', help='POSIX pseudo-terminal render and resize evidence')
    parser.add_argument('--tty', help='Explicit existing terminal path for user-visible display')
    parser.add_argument('--serve', action='store_true')
    parser.add_argument('--artifacts', type=Path)
    args = parser.parse_args()
    folder = args.artifacts.resolve() if args.artifacts else Path(tempfile.mkdtemp(prefix='log-print-ui-'))
    folder.mkdir(parents=True, exist_ok=True)
    print('Artifacts:', folder, flush=True)
    state = folder / 'state.json'
    if state.exists():
        raise RuntimeError('Refusing to reuse an existing state file')
    suffix = '.exe' if os.name == 'nt' else ''
    exe = ROOT / 'target' / 'debug' / ('log-print' + suffix)
    if not args.skip_build:
        subprocess.run(['cargo', 'build', '-p', 'app-log-print', '-p', 'log-core', '-p', 'input-file', '-p', 'output-tui', '-p', 'output-webui'], cwd=ROOT, check=True)
    source = folder / 'source.log'
    source.write_bytes(b'')
    tty = args.tty
    master = slave = None
    terminal_bytes = bytearray()
    terminal_stop = threading.Event()
    if args.pty:
        if os.name != 'posix':
            raise RuntimeError('--pty is only available on POSIX; Windows run is headless unless attached to a foreground console')
        import pty
        import fcntl
        import termios
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 32, 120, 0, 0))
        tty = os.ttyname(slave)
        def drain():
            while not terminal_stop.is_set():
                if select.select([master], [], [], 0.1)[0]:
                    try:
                        chunk = os.read(master, 65536)
                    except OSError:
                        break
                    terminal_bytes.extend(chunk)
                    if len(terminal_bytes) > 2_000_000:
                        del terminal_bytes[:-2_000_000]
        threading.Thread(target=drain, daemon=True).start()
    series = [dict(name='Temperature', stream='sensor', pattern=r'temp=(?P<value>[-+0-9.eE]+)'), dict(name='Load', stream='sensor', pattern=r'load=(?P<value>[-+0-9.eE]+)')]
    sessions = [dict(id='alpha', title='Device telemetry', series=series, window_secs=60), dict(id='beta', title='Independent session', series=series[:1], window_secs=60)]
    ui_config = dict(streams=['sensor'], sessions=sessions)
    config = {'plugins': [
        dict(id='source', bin='input-file', streams=[{'id': 'sensor'}], config=dict(path=str(source), stream='sensor', from_start=True, poll_ms=10)),
        dict(id='tui', bin='output-tui', reads=['sensor'], config={**ui_config, 'headless': not bool(tty), 'tty': tty}),
        dict(id='web', bin='output-webui', reads=['sensor'], config=ui_config),
    ]}
    config_path = folder / 'config.json'
    config_path.write_text(json.dumps(config, indent=2), encoding='utf-8')
    commands = []
    def cli(*parts, success=True):
        result = subprocess.run([str(exe), '--state', str(state), *parts], cwd=ROOT, text=True, encoding='utf-8', capture_output=True, timeout=35)
        commands.append({'args': list(parts), 'exit': result.returncode})
        if success and result.returncode:
            raise RuntimeError(f'{parts}: {result.stderr}')
        return json.loads(result.stdout) if success else result
    def get(plugin, id='alpha'):
        return cli('session', 'get', plugin, id)
    def append(data):
        with source.open('ab') as file:
            file.write(data)
            file.flush()
    def until(fn, timeout=10):
        end = time.monotonic() + timeout
        last = None
        while time.monotonic() < end:
            try:
                last = fn()
                if last:
                    return last
            except Exception as error:
                last = str(error)
            time.sleep(0.05)
        raise AssertionError(f'timed out; last={last}')
    def sse_snapshot(url):
        with urllib.request.urlopen(url + '/api/sessions/alpha/events', timeout=5) as response:
            for raw in response:
                if raw.startswith(b'data:'):
                    return json.loads(raw[5:])
        raise AssertionError('No SSE snapshot')
    report = {'platform': sys.platform, 'artifacts': str(folder), 'checks': [], 'browser_review': 'not performed by this script', 'visible_terminal_review': bool(args.tty), 'terminal_mode': 'pty' if args.pty else 'explicit_tty' if args.tty else 'headless'}
    def passed(name):
        report['checks'].append(name)
        print('PASS', name, flush=True)
    try:
        cli('start', '--config', str(config_path))
        def web_url():
            return next((p.get('report', {}).get('url') for p in cli('status')['plugins'] if p['id'] == 'web'), None)
        url = until(web_url)
        until(lambda: get('tui'))
        append(b'temp=1')
        time.sleep(0.1)
        append(b'2.5 load=4\n')
        until(lambda: get('web')['series'][0]['matched'] == 1)
        for plugin in ('web', 'tui'):
            assert get(plugin)['series'][0]['data'][0][1] == 12.5
        passed('same real stream reaches both plugins; number split across source writes becomes 12.5')
        for n in range(60):
            append(f'temp={22+6*math.sin(n/8):.3f} load={42+10*math.cos(n/6):.3f}\n'.encode())
            time.sleep(0.015)
        until(lambda: get('web')['series'][0]['matched'] >= 61)
        selection = cli('session', 'select', 'web', 'beta')
        assert selection['url'] == url + '/?session=beta'
        initial = sse_snapshot(url)
        append(b'temp=29 load=51\n')
        until(lambda: get('web')['generation'] > initial['generation'])
        assert sse_snapshot(url)['generation'] > initial['generation']
        passed('HTTP SSE delivers increasing live snapshots')
        before_beta = get('web', 'beta')
        before_tui = get('tui')
        current = get('web')
        cli('session', 'set', 'web', 'alpha', '--revision', str(current['revision']), '--json', json.dumps({'title': 'Telemetry · 温度', 'theme': 'light', 'window_secs': 10}))
        after = get('web')
        assert after['title'] == 'Telemetry · 温度' and after['window_secs'] == 10
        assert get('web', 'beta')['revision'] == before_beta['revision']
        assert get('tui')['revision'] == before_tui['revision']
        stale = cli('session', 'set', 'web', 'alpha', '--revision', str(current['revision']), '--json', '{"title":"stale"}', success=False)
        assert stale.returncode and 'revision_conflict' in stale.stderr
        passed('CLI revision patch changes only selected session; stale writes rejected')
        for plugin in ('web', 'tui'):
            current = get(plugin)
            for format in ('png', 'svg'):
                path = folder / f'{plugin}.{format}'
                cli('session', 'export', plugin, 'alpha', '--path', str(path), '--format', format, '--revision', str(current['revision']))
                meta = json.loads(path.with_suffix('.' + format + '.json').read_text(encoding='utf-8'))
                assert meta['revision'] == current['revision'] and meta['config']['id'] == 'alpha'
                assert meta['source_status']['sensor']['epoch']
                if format == 'png':
                    raw = path.read_bytes()
                    assert raw[:8] == b'\x89PNG\r\n\x1a\n' and struct.unpack('>II', raw[16:24]) == (1200, 700)
                else:
                    text = path.read_text(encoding='utf-8')
                    assert '<svg' in text and 'data:font/otf;base64,' in text
        passed('both plugins export PNG/SVG with frozen revision, source metadata and embedded font')
        if args.pty:
            until(lambda: b'LOG_PRINT' in terminal_bytes)
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 80, 0, 0))
            time.sleep(0.3)
            tui = get('tui')
            cli('session', 'set', 'tui', 'alpha', '--revision', str(tui['revision']), '--json', '{"title":"TTY_UPDATED"}')
            until(lambda: b'TTY_UPDATED' in terminal_bytes)
            cli('session', 'select', 'tui', 'beta')
            time.sleep(0.2)
            # A resize requests a full frame; diff rendering legitimately omits unchanged spaces.
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 28, 100, 0, 0))
            until(lambda: b'Independent session' in terminal_bytes)
            cli('session', 'select', 'tui', 'alpha')
            passed('PTY real ANSI chart rendered, resized and changed by session CLI')
        html = urllib.request.urlopen(url, timeout=5).read().decode()
        resources = re.findall(r'(?:src|href)="([^"]+)"', html)
        assert all(item.startswith('/') for item in resources)
        for resource in ('/assets/echarts.min.js', '/assets/app.js', '/assets/style.css'):
            assert urllib.request.urlopen(url + resource, timeout=5).status == 200
        passed('HTML and all requested UI assets served from local process')
        report.update(url=url, state=str(state), commands=commands, successful=True)
        (folder / 'report.json').write_text(json.dumps(report, indent=2, ensure_ascii=False), encoding='utf-8')
        print(json.dumps({'ready': True, 'url': url, 'state': str(state), 'source': str(source), 'artifacts': str(folder)}, ensure_ascii=False), flush=True)
        if args.serve:
            print('LIVE_REVIEW: press Ctrl-C to stop only this test instance', flush=True)
            n = 60
            while True:
                append(f'temp={22+6*math.sin(n/14):.3f} load={42+10*math.cos(n/19):.3f}\n'.encode())
                n += 1
                time.sleep(0.1)
    except KeyboardInterrupt:
        pass
    finally:
        if state.exists():
            try:
                cli('stop')
                report['stopped'] = not state.exists()
            except Exception as error:
                report['cleanup_error'] = str(error)
        terminal_stop.set()
        time.sleep(0.15)
        if master is not None:
            (folder / 'tui.ansi').write_bytes(terminal_bytes)
            os.close(master)
            os.close(slave)
        report['commands'] = commands
        (folder / 'report.json').write_text(json.dumps(report, indent=2, ensure_ascii=False), encoding='utf-8')
    print('Artifacts:', folder)


if __name__ == '__main__':
    main()
