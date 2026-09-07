#!/usr/bin/env python3
"""Run a Python process test with interrupt-driven finally cleanup on Windows too."""
import os
from pathlib import Path
import runpy
import signal
import sys


if len(sys.argv) < 2:
    raise SystemExit('usage: python ci/python-test.py TEST_SCRIPT [ARGUMENTS...]')
if os.name == 'nt':
    # The coordinator sends CTRL_BREAK_EVENT to its owned process group. Python's
    # default SIGBREAK action terminates without running the test's finally block.
    signal.signal(signal.SIGBREAK, signal.default_int_handler)
script = Path(sys.argv[1]).resolve()
sys.argv = [str(script), *sys.argv[2:]]
sys.path.insert(0, str(script.parent))
runpy.run_path(str(script), run_name='__main__')
