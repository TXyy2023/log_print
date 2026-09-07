#!/usr/bin/env python3
"""Cross-platform formal validation coordinator. Python 3.12+, stdlib only."""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import platform
import shlex
import shutil
import signal
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]


def command_text(command):
    return subprocess.list2cmdline(command) if os.name == 'nt' else shlex.join(command)


def probe(command):
    try:
        result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, encoding='utf-8', errors='replace', timeout=20)
        return {'command': command, 'returncode': result.returncode, 'stdout': result.stdout.strip(), 'stderr': result.stderr.strip()}
    except (OSError, subprocess.TimeoutExpired) as error:
        return {'command': command, 'error': str(error)}


def stop_owned(process):
    """Give the test harness a chance to execute its own finally cleanup first."""
    if process.poll() is not None:
        return
    try:
        if os.name == 'nt':
            process.send_signal(signal.CTRL_BREAK_EVENT)
        else:
            os.killpg(process.pid, signal.SIGINT)
        process.wait(timeout=40)
    except (OSError, subprocess.TimeoutExpired):
        if process.poll() is None:
            if os.name == 'nt':
                process.kill()
            else:
                os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=10)


def execute(name, command, report_dir, timeout):
    started = datetime.now(timezone.utc).isoformat()
    before = time.monotonic()
    log_path = report_dir / f'{name}.log'
    result = {'name': name, 'command': command, 'cwd': str(ROOT), 'started': started, 'log': log_path.name}
    print(f'RUN {name}: {command_text(command)}', flush=True)
    with log_path.open('wb') as log:
        process = None
        try:
            options = {'cwd': ROOT, 'stdout': log, 'stderr': subprocess.STDOUT,
                       'env': dict(os.environ, PYTHONUTF8='1', PYTHONIOENCODING='utf-8')}
            if os.name == 'nt':
                options['creationflags'] = subprocess.CREATE_NEW_PROCESS_GROUP
            else:
                options['start_new_session'] = True
            process = subprocess.Popen(command, **options)
            result['returncode'] = process.wait(timeout=timeout)
            result['status'] = 'pass' if result['returncode'] == 0 else 'fail'
        except subprocess.TimeoutExpired:
            stop_owned(process)
            result.update(status='fail', returncode=124, error=f'timed out after {timeout}s; requested owned test harness cleanup')
        except KeyboardInterrupt:
            if process is not None:
                stop_owned(process)
            result.update(status='interrupted', returncode=130, error='interrupted; requested owned test harness cleanup')
        except OSError as error:
            result.update(status='fail', returncode=127, error=str(error))
    result['seconds'] = round(time.monotonic() - before, 3)
    print(f'{result["status"].upper()} {name}: code={result["returncode"]}, {result["seconds"]}s, log={log_path}', flush=True)
    # Keep both a complete artifact and useful terminal output on failures.
    if result['status'] != 'pass':
        tail = log_path.read_text(encoding='utf-8', errors='replace').splitlines()[-50:]
        print('\n'.join(tail), flush=True)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--report-dir', type=Path)
    parser.add_argument('--timeout', type=int, default=1800, help='Per-command timeout in seconds')
    parser.add_argument('--skip-build', action='store_true', help='Iteration only: skip all Cargo checks/build; report remains partial')
    parser.add_argument('--list', action='store_true', help='Print the selected commands without running or claiming validation')
    args = parser.parse_args()
    if args.timeout < 1:
        parser.error('--timeout must be positive')
    stamp = datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%SZ')
    report_dir = (args.report_dir or ROOT / 'artifacts/validation' / f'{stamp}-{platform.system().lower()}-{os.getpid()}').resolve()
    def python_test(path, *arguments):
        return [sys.executable, str(ROOT / 'ci/python-test.py'), str(ROOT / path), *arguments]
    steps = [
        ('fmt', ['cargo', 'fmt', '--all', '--check']),
        ('clippy', ['cargo', 'clippy', '--workspace', '--all-targets', '--locked', '--', '-D', 'warnings']),
        ('rust-tests', ['cargo', 'test', '--workspace', '--locked']),
        ('build', ['cargo', 'build', '--workspace', '--locked']),
        ('protocol', python_test('tests/protocol.py', '--report', str(report_dir / 'protocol-results.json'))),
        ('supervisor', python_test('crates/app-log-print/tests/supervisor.py')),
        ('reliability-review', python_test('crates/app-log-print/tests/reliability_review.py', '--report', str(report_dir / 'reliability-review-results.json'))),
    ]
    ui_command = python_test('plugins/outputs/output-webui/tests/verify_ui.py', '--skip-build', '--artifacts', str(report_dir / 'ui'))
    if os.name == 'posix':
        ui_command.append('--pty')
    steps.extend([
        ('io', python_test('examples/io/verify.py')),
        ('languages', python_test('examples/io/languages.py', '--json', str(report_dir / 'input-languages.json'))),
        ('ui', ui_command),
    ])
    optional = []
    if args.list:
        for name, command in steps:
            print(f'{name}: {command_text(command)}')
        return 0
    report_dir.mkdir(parents=True, exist_ok=False)
    environment = {
        'timestamp': datetime.now(timezone.utc).isoformat(),
        'platform': platform.platform(), 'system': platform.system(), 'architecture': platform.machine(),
        'python': sys.version, 'python_executable': sys.executable,
        'python_test_environment': {'PYTHONUTF8': '1', 'PYTHONIOENCODING': 'utf-8'},
        'cargo_path': shutil.which('cargo'), 'rustc_path': shutil.which('rustc'),
        'rustc': probe(['rustc', '-Vv']), 'cargo': probe(['cargo', '-V']),
        'git_head': probe(['git', 'rev-parse', 'HEAD']),
        'ci': {key: os.environ[key] for key in ('CI', 'GITHUB_ACTIONS', 'RUNNER_OS', 'RUNNER_ARCH', 'LOG_PRINT_PROFILE') if key in os.environ},
    }
    (report_dir / 'environment.json').write_text(json.dumps(environment, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    results = []
    if sys.version_info < (3, 12):
        results.append({'name': 'python', 'status': 'fail', 'returncode': 1, 'error': 'Python 3.12 or newer is required'})
    else:
        build_failed = False
        for name, command in steps:
            if args.skip_build and name in ('fmt', 'clippy', 'rust-tests', 'build'):
                results.append({'name': name, 'status': 'skip', 'reason': '--skip-build is iteration only'})
                continue
            if build_failed:
                results.append({'name': name, 'status': 'skip', 'reason': 'workspace build failed; stale binaries must not be accepted'})
                continue
            result = execute(name, command, report_dir, args.timeout)
            results.append(result)
            (report_dir / 'results.json').write_text(json.dumps({'environment': environment, 'results': results, 'optional': optional}, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
            if name == 'build' and result['status'] != 'pass':
                build_failed = True
            if result['status'] == 'interrupted':
                break
    failed = any(row['status'] in ('fail', 'interrupted') for row in results)
    complete = not failed and not args.skip_build and len(results) == len(steps) and all(row['status'] == 'pass' for row in results)
    summary = {'environment': environment, 'complete_selected_suite': complete, 'iteration_only': args.skip_build, 'results': results, 'optional': optional,
               'boundaries': ['This host only; does not establish other-platform execution.', 'Headless/UI test success does not establish an actual desktop visual review.', 'No real-device, power-loss, or benchmark results are inferred from this suite.']}
    (report_dir / 'results.json').write_text(json.dumps(summary, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    lines = ['# Validation result', '', f'Platform: {environment["platform"]}', f'Python: {sys.executable}', f'Complete selected suite: {complete}', '', '| Step | Status | Exit | Seconds | Log |', '|---|---|---:|---:|---|']
    for row in results:
        lines.append(f'| {row["name"]} | {row["status"]} | {row.get("returncode", "")} | {row.get("seconds", "")} | {row.get("log", row.get("reason", ""))} |')
    lines.extend(['', *summary['boundaries']])
    (report_dir / 'summary.md').write_text('\n'.join(lines) + '\n', encoding='utf-8')
    print(f'REPORT {report_dir / "results.json"}', flush=True)
    return 1 if failed else 0


if __name__ == '__main__':
    raise SystemExit(main())
