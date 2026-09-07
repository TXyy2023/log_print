#!/usr/bin/env python3
"""Bounded-duration real TUI + WebUI exercise, with isolated Chrome resource sampling.

Requires a prebuilt release workspace and an isolated Python with psutil and
websocket-client. No existing browser profile is opened. Default: 180 s, 1000 LF
numeric lines/s, 100 ms display cadence, 2 sessions/plugin and 2 curves/session.
"""
import argparse
import base64
import json
import math
import os
from pathlib import Path
import platform
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
import urllib.request

ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / 'tests'))
import benchmark


def emit(args):
    folder = args.source_dir
    (folder / 'ready').write_text(str(os.getpid()), encoding='utf-8')
    benchmark.wait(lambda: (folder / 'gate').exists(), 'producer gate timeout', 60)
    begin = time.perf_counter()
    count = round(args.duration * args.rate)
    with (folder / 'source.log').open('ab', buffering=0) as target:
        for n in range(count):
            delay = begin + n / args.rate - time.perf_counter()
            if delay > 0:
                time.sleep(delay)
            payload = f'index={n} temp={22+6*math.sin(n/1400):.4f} load={42+10*math.cos(n/1900):.4f}\n'.encode()
            assert target.write(payload) == len(payload)
            if n % 100 == 0:
                temporary = folder / 'progress.pending'
                temporary.write_text(json.dumps({'lines': n + 1, 'elapsed_s': time.perf_counter() - begin}), encoding='utf-8')
                temporary.replace(folder / 'progress.json')
    elapsed = time.perf_counter() - begin
    (folder / 'source-result.json').write_text(json.dumps({'lines': count, 'duration_s': elapsed, 'lines_per_s': count / elapsed}), encoding='utf-8')


class Browser:
    def __init__(self, chrome, folder, url):
        import websocket
        self.profile = folder / 'chrome-profile'
        self.log = (folder / 'chrome.stderr.log').open('wb')
        self.process = subprocess.Popen([str(chrome), '--headless=new', '--no-first-run',
            '--no-default-browser-check', '--disable-background-networking',
            '--disable-component-update', '--disable-sync', '--window-size=1280,900',
            '--remote-debugging-port=0', '--user-data-dir=' + str(self.profile), url],
            stdout=subprocess.DEVNULL, stderr=self.log)
        portfile = self.profile / 'DevToolsActivePort'
        benchmark.wait(portfile.exists, 'isolated Chrome DevTools not ready', 20)
        port = int(portfile.read_text(encoding='utf-8').splitlines()[0])
        def page():
            with urllib.request.urlopen(f'http://127.0.0.1:{port}/json', timeout=3) as response:
                return next((p for p in json.load(response) if p['type'] == 'page' and p['url'].startswith(url)), None)
        endpoint = benchmark.wait(page, 'isolated Chrome page missing')
        self.socket = websocket.create_connection(endpoint['webSocketDebuggerUrl'], timeout=5, suppress_origin=True)
        self.next_id = 0
        self.call('Runtime.enable')
        benchmark.wait(lambda: self.state().get('id') == 'alpha', 'real browser page failed to render', 20)
        self.version = self.call('Browser.getVersion')

    def call(self, method, params=None):
        self.next_id += 1
        self.socket.send(json.dumps({'id': self.next_id, 'method': method, 'params': params or {}}))
        while True:
            reply = json.loads(self.socket.recv())
            if reply.get('id') == self.next_id:
                if 'error' in reply:
                    raise RuntimeError(reply['error'])
                return reply['result']

    def state(self):
        expression = "({id:current?.id, revision:current?.revision, generation:current?.generation, title:document.getElementById('title').textContent, series:chart?.getOption().series.map(s=>({name:s.name,points:s.data.length})), canvases:document.querySelectorAll('canvas').length, connected:document.getElementById('connection').textContent, error:document.getElementById('error').hidden?null:document.getElementById('error').textContent})"
        result = self.call('Runtime.evaluate', {'expression': expression, 'returnByValue': True})
        return result.get('result', {}).get('value', {})

    def screenshot(self, path):
        image = self.call('Page.captureScreenshot', {'format': 'png'})
        path.write_bytes(base64.b64decode(image['data']))

    def close(self):
        import psutil
        try:
            children = psutil.Process(self.process.pid).children(recursive=True)
        except psutil.Error:
            children = []
        try:
            self.call('Browser.close')
        except Exception:
            pass
        try:
            self.process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.process.terminate()
            self.process.wait(timeout=5)
        self.socket.close()
        self.log.close()
        _, alive = psutil.wait_procs(children, timeout=5)
        for child in alive:
            child.terminate()
        _, alive = psutil.wait_procs(alive, timeout=3)
        if alive:
            raise RuntimeError('isolated Chrome children did not stop')
        # The profile is disposable and contains no user login; keep reports, not cache files.
        shutil.rmtree(self.profile)


def resource_summary(data):
    rows = data.pop('raw_samples')
    start = rows[0]['time_ns']
    stable = [row for row in rows if (row['time_ns'] - start) / 1e9 >= 30]
    if stable:
        x = [(row['time_ns'] - start) / 1e9 for row in stable]
        y = [row['tree_rss_bytes'] / 1048576 for row in stable]
        mean_x, mean_y = statistics.mean(x), statistics.mean(y)
        denominator = sum((value - mean_x) ** 2 for value in x)
        slope = sum((a - mean_x) * (b - mean_y) for a, b in zip(x, y)) / denominator if denominator else None
        data['after_30s'] = {'first_rss_mib': y[0], 'last_rss_mib': y[-1],
            'min_rss_mib': min(y), 'max_rss_mib': max(y), 'linear_rss_mib_per_min': slope * 60 if slope is not None else None}
    return data, rows


def run(args):
    if os.name != 'posix':
        raise RuntimeError('this sustained PTY harness requires POSIX; no Windows result is implied')
    import psutil
    import websocket  # Check tools before starting any application process.
    folder = args.artifacts.resolve() if args.artifacts else Path(tempfile.mkdtemp(prefix='log-print-ui-sustain-'))
    folder.mkdir(parents=True, exist_ok=True)
    state = folder / 'state.json'
    if state.exists() or (folder / 'gate').exists():
        raise RuntimeError('use a fresh artifacts directory')
    (folder / 'source.log').write_bytes(b'')
    exe = ROOT / 'target' / 'release' / 'log-print'
    terminal = benchmark.Terminal()
    source = browser = None
    samplers = {}
    commands, snapshots, changes = [], [], []
    report = {'environment': {'platform': platform.platform(), 'arch': platform.machine(),
        'python': platform.python_version(), 'psutil': psutil.__version__, 'websocket_client': websocket.__version__},
        'duration_target_s': args.duration, 'rate_target_lines_s': args.rate, 'build': 'release',
        'mode': 'real PTY + local WebUI + independent headless Chrome profile',
        'limits': {'sessions_per_plugin': 2, 'curves_per_session': 2, 'retained_points_per_curve': 2048,
            'display_points_per_curve': 512, 'sdk_event_queue_record_capacity': 64,
            'queue_occupancy': 'not exposed; retained samples and complete-line progress measured, not transport queue occupancy'},
        'scope': 'bounded duration only; not a maximum stable rate or multi-hour stability claim; headless Chrome is separate from prior visible Chrome review'}
    def cli(*parts):
        start = time.perf_counter()
        result = subprocess.run([str(exe), '--state', str(state), *parts], capture_output=True, text=True, encoding='utf-8', timeout=20)
        commands.append({'args': list(parts), 'elapsed_ms': (time.perf_counter() - start) * 1000, 'exit': result.returncode})
        if result.returncode:
            raise RuntimeError(f'CLI {parts}: {result.stderr}')
        return json.loads(result.stdout)
    def snapshot(plugin, session):
        return cli('session', 'get', plugin, session)
    def metrics(value):
        return {'id': value['id'], 'revision': value['revision'], 'generation': value['generation'],
            'config': value['config'], 'source': value['live_source_status']['sensor'],
            'series': [{k: curve[k] for k in ('name', 'retained_points', 'matched', 'unmatched', 'invalid')} |
                {'displayed_points': len(curve['data'])} for curve in value['series']]}
    print('Artifacts:', folder, flush=True)
    try:
        series = [dict(name='Temperature', stream='sensor', pattern=r'temp=(?P<value>[-+0-9.eE]+)'),
                  dict(name='Load', stream='sensor', pattern=r'load=(?P<value>[-+0-9.eE]+)')]
        sessions = [dict(id=id, title='Sustained telemetry ' + id, series=series, window_secs=60, max_points=2048, refresh_ms=100) for id in ('alpha', 'beta')]
        ui = dict(streams=['sensor'], sessions=sessions)
        config = {'plugins': [
            dict(id='source', bin='input-file', streams=[{'id': 'sensor'}], config=dict(path=str(folder / 'source.log'), stream='sensor', from_start=True, poll_ms=5)),
            dict(id='tui', bin='output-tui', reads=['sensor'], config={**ui, 'tty': terminal.path}),
            dict(id='web', bin='output-webui', reads=['sensor'], config=ui)]}
        config_path = folder / 'config.json'
        config_path.write_text(json.dumps(config, indent=2), encoding='utf-8')
        cli('start', '--config', str(config_path))
        status = benchmark.wait(lambda: cli('status'), 'status unavailable')
        def web_url():
            return next((p.get('report', {}).get('url') for p in cli('status')['plugins'] if p['id'] == 'web'), None)
        url = benchmark.wait(web_url, 'WebUI not ready')
        benchmark.wait(lambda: snapshot('tui', 'alpha'), 'TUI not ready')
        browser = Browser(args.chrome, folder, url)
        report['browser_version'] = browser.version
        browser.screenshot(folder / 'browser-start.png')
        original_beta = {plugin: snapshot(plugin, 'beta')['config'] for plugin in ('tui', 'web')}
        source = subprocess.Popen([sys.executable, str(Path(__file__).resolve()), '--source-dir', str(folder), '--duration', str(args.duration), '--rate', str(args.rate)], stdout=subprocess.DEVNULL)
        benchmark.wait((folder / 'ready').exists, 'producer not ready')
        # Separate process samplers keep psutil scans out of the producer and UI clients.
        supervisor_pid = status['supervisor']['pid']
        for name, pid in (('plugin_tree', supervisor_pid), ('browser_tree', browser.process.pid)):
            directory = folder / name
            directory.mkdir()
            samplers[name] = benchmark.SamplerProcess([pid], directory)
        report['roots'] = {'supervisor_pid': supervisor_pid, 'browser_pid': browser.process.pid, 'producer_pid_excluded': source.pid}
        started = time.perf_counter()
        (folder / 'gate').write_text('go', encoding='utf-8')
        step = 0
        while True:
            target = started + step * 10
            delay = target - time.perf_counter()
            if delay > 0:
                time.sleep(delay)
            elapsed = time.perf_counter() - started
            row = {'elapsed_s': elapsed, 'plugins': {}, 'browser': browser.state()}
            for plugin in ('tui', 'web'):
                row['plugins'][plugin] = {}
                for session in ('alpha', 'beta'):
                    value = snapshot(plugin, session)
                    row['plugins'][plugin][session] = metrics(value)
                    for curve in value['series']:
                        assert curve['retained_points'] <= 2048 and len(curve['data']) <= 512
                    assert value['live_source_status']['sensor']['gaps'] == 0
                    assert not value['live_source_status']['sensor']['disconnected']
                assert row['plugins'][plugin]['beta']['revision'] == 1
                assert row['plugins'][plugin]['beta']['config'] == original_beta[plugin]
            assert row['browser'].get('error') is None
            assert row['browser']['canvases'] > 0 and len(row['browser']['series']) == 2
            assert all(curve['points'] <= 512 for curve in row['browser']['series'])
            snapshots.append(row)
            print(json.dumps({'elapsed_s': round(elapsed, 1), 'tui_lines': row['plugins']['tui']['alpha']['source']['lines'],
                'web_lines': row['plugins']['web']['alpha']['source']['lines'], 'browser_generation': row['browser']['generation']}), flush=True)
            if elapsed >= args.duration:
                break
            if step and step % 2 == 0:
                plugin = 'web' if step % 4 == 2 else 'tui'
                other = 'tui' if plugin == 'web' else 'web'
                before_other = snapshot(other, 'alpha')['revision']
                revision = row['plugins'][plugin]['alpha']['revision']
                patch = {'title': f'Sustained {plugin} cycle {step}', 'window_secs': 20 if step % 4 == 2 else 60}
                cli('session', 'set', plugin, 'alpha', '--revision', str(revision), '--json', json.dumps(patch))
                assert snapshot(plugin, 'alpha')['revision'] == revision + 1
                assert snapshot(other, 'alpha')['revision'] == before_other
                if plugin == 'web':
                    benchmark.wait(lambda: browser.state().get('revision') == revision + 1, 'browser missed live CLI revision')
                changes.append({'elapsed_s': elapsed, 'plugin': plugin, 'new_revision': revision + 1, 'other_revision_unchanged': before_other})
            step += 1
        source.wait(timeout=10)
        assert source.returncode == 0
        expected = round(args.duration * args.rate)
        for plugin in ('tui', 'web'):
            benchmark.wait(lambda: snapshot(plugin, 'alpha')['live_source_status']['sensor']['lines'] == expected, 'complete lines not delivered', 15)
        report['final'] = {plugin: {session: metrics(snapshot(plugin, session)) for session in ('alpha', 'beta')} for plugin in ('tui', 'web')}
        for plugin in report['final'].values():
            for session in plugin.values():
                assert session['source']['lines'] == expected
                assert session['source']['invalid_utf8'] == session['source']['oversized_lines'] == 0
                assert all(curve['matched'] == expected and curve['invalid'] == 0 for curve in session['series'])
        report['source'] = json.loads((folder / 'source-result.json').read_text(encoding='utf-8'))
        browser.screenshot(folder / 'browser-final.png')
        report['successful'] = True
    finally:
        report['resources'] = {}
        for name, sampler in samplers.items():
            summary, raw = resource_summary(sampler.finish())
            report['resources'][name] = summary
            (folder / (name + '.samples.json')).write_text(json.dumps(raw), encoding='utf-8')
        if source and source.poll() is None:
            source.terminate()
            source.wait(timeout=5)
        if browser:
            browser.close()
            report['isolated_browser_stopped'] = True
        if state.exists():
            cli('stop')
            report['instance_stopped'] = not state.exists()
        report['pty_bytes_rendered'] = terminal.bytes
        terminal.close()
        report['snapshots'] = snapshots
        report['session_changes'] = changes
        report['commands'] = commands
        if commands:
            latencies = [command['elapsed_ms'] for command in commands if command['args'][0] == 'session']
            report['cli_session_wall_latency_ms'] = {'p50': benchmark.percentile(latencies, 50), 'p95': benchmark.percentile(latencies, 95), 'max': max(latencies)}
        (folder / 'report.json').write_text(json.dumps(report, indent=2, ensure_ascii=False), encoding='utf-8')
    print(json.dumps({'successful': report['successful'], 'artifacts': str(folder), 'resources': report['resources'], 'cli': report['cli_session_wall_latency_ms']}, ensure_ascii=False), flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--duration', type=float, default=180)
    parser.add_argument('--rate', type=float, default=1000)
    parser.add_argument('--artifacts', type=Path)
    parser.add_argument('--source-dir', type=Path)
    parser.add_argument('--chrome', type=Path, default=Path('/Applications/Google Chrome.app/Contents/MacOS/Google Chrome'))
    options = parser.parse_args()
    if not 1 <= options.duration <= 600 or not 1 <= options.rate <= 10000:
        parser.error('bounded run requires duration 1..600 seconds and rate 1..10000 lines/s')
    emit(options) if options.source_dir else run(options)
