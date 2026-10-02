#!/usr/bin/env python3
"""Build and test the installed Python SDK wheel, outside the source checkout."""
import json
import os
import shutil
from pathlib import Path
import subprocess
import sys
import tempfile
import venv

ROOT = Path(__file__).resolve().parents[2]
SDK = ROOT / 'project/sdks/python'
ARTIFACTS = ROOT / 'quality/artifacts/python-sdk'


def main():
    ARTIFACTS.mkdir(parents=True, exist_ok=True)
    results = []
    with tempfile.TemporaryDirectory(prefix='sdk acceptance ') as tmp:
        tmp = Path(tmp)
        def run(name, args, **kwargs):
            with (ARTIFACTS / f'{name}.log').open('w', encoding='utf-8') as log:
                result = subprocess.run(args, stdout=log, stderr=subprocess.STDOUT,
                                        timeout=600, **kwargs)
            results.append({'name': name, 'exit': result.returncode})
            print((ARTIFACTS / f'{name}.log').read_text(encoding='utf-8'), flush=True)
            (ARTIFACTS / 'results.json').write_text(json.dumps(results, indent=2))
            if result.returncode:
                raise SystemExit(result.returncode)
        run('wheel', [sys.executable, '-m', 'pip', 'wheel', '--no-deps',
                      '--wheel-dir', str(tmp / 'wheels'), str(SDK)])
        venv.EnvBuilder(with_pip=True, symlinks=os.name != 'nt').create(tmp / 'venv')
        python = tmp / 'venv' / ('Scripts/python.exe' if os.name == 'nt' else 'bin/python')
        wheel, = (tmp / 'wheels').glob('*.whl')
        shutil.copy2(wheel, ARTIFACTS / wheel.name)
        run('install', [str(python), '-m', 'pip', 'install', '--no-index', '--no-deps', str(wheel)])
        env = dict(os.environ, LOG_PRINT_SDK_TEST_ROOT=str(ROOT),
                   LOG_PRINT_CORE_BIN=str(ROOT / 'target/debug' / ('log-print-core.exe' if os.name == 'nt' else 'log-print-core')),
                   PYTHONUTF8='1', PYTHONIOENCODING='utf-8')
        env.pop('PYTHONPATH', None)
        for key in ('LOG_PRINT_CORE', 'LOG_PRINT_PLUGIN', 'LOG_PRINT_TOKEN', 'LOG_PRINT_CONFIG', 'LOG_PRINT_TRANSPORT'):
            env.pop(key, None)
        run('tests', [str(python), '-W', 'error::ResourceWarning', '-m', 'unittest', 'discover',
                      '-s', str(SDK / 'tests'), '-v'], cwd=tmp, env=env)
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
