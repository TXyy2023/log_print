#!/usr/bin/env python3
"""Supervisor failure reporting and per-launch readiness, using real protocol peers."""
import json
from pathlib import Path
import sys
import tempfile
import unittest

from support import App, eventually


# This is a deliberate fault-injection peer, not a genuine software log fixture.
# It registers using the ordinary protocol and fails only after accepting stop.
STUB = r'''
import json
import os
from pathlib import Path
import socket
import sys

config = json.loads(os.environ['LOG_PRINT_CONFIG'])
marker = Path(config['marker'])
previous = marker.read_text().splitlines() if marker.exists() else []
with marker.open('a') as output:
    output.write(str(os.getpid()) + '\n')
if config.get('terminal_once') and previous:
    # A new process that exits successfully without registering is NOT ready.
    sys.exit(0)

host, port = os.environ['LOG_PRINT_CORE'].rsplit(':', 1)
socket_ = socket.create_connection((host, int(port)))
reader = socket_.makefile('rb')
def send(value):
    socket_.sendall(json.dumps(value).encode() + b'\n')
def receive():
    data = reader.readline()
    if not data:
        sys.exit(3)
    return json.loads(data)

send({'protocol':'log-print/2', 'plugin':os.environ['LOG_PRINT_PLUGIN'],
      'token':os.environ['LOG_PRINT_TOKEN'], 'events':False})
assert not receive().get('error')
state = 'source_eof' if config.get('terminal_once') else 'capturing'
send({'id':1, 'op':'report', 'args':{'state':state, 'run_pid':os.getpid()}})
assert not receive().get('error')
if config.get('terminal_once'):
    sys.exit(0)
while True:
    message = receive()
    if message.get('type') == 'control' and message['method'] == 'shutdown':
        send({'id':2, 'op':'reply', 'args':{'call_id':message['call_id'],
              'result':{'stopping':True, 'completed':False}, 'error':None}})
        receive()
        # Model a flush/cleanup failure after the accepted shutdown request.
        sys.exit(7)
'''


def config(marker, terminal_once=False):
    return {'plugins':[{'id':'probe', 'role':'input', 'bin':sys.executable,
                        'args':['-c', STUB], 'streams':[{'id':'probe-log'}],
                        'config':{'marker':str(marker), 'terminal_once':terminal_once}}]}


class FailureApp(App):
    """Accept the intentional child failure while still requiring full cleanup."""
    def __exit__(self, *args):
        try:
            if self.started and self.state.exists():
                self.cli('stop', ok=False)
                eventually(lambda:not self.state.exists())
        finally:
            self.temp.cleanup()


class SupervisorFailureTests(unittest.TestCase):
    def test_plugin_stop_returns_nonzero_for_nonzero_child_exit(self):
        with tempfile.TemporaryDirectory() as td:
            marker = Path(td)/'runs.txt'
            with FailureApp(config(marker)) as app:
                result = app.cli('plugin', 'stop', 'probe', ok=False)
                self.assertNotEqual(result.returncode, 0, result.stdout)
                report = json.loads(result.stdout)
                self.assertFalse(report['success'])
                self.assertIn('7', report['exit'])
                self.assertFalse(report['forced'])
                self.assertEqual(len(marker.read_text().splitlines()), 1)
                self.assertEqual(app.json('status')['plugin_processes'][0]['state'], 'stopped')

    def test_restart_does_not_launch_after_failed_shutdown(self):
        with tempfile.TemporaryDirectory() as td:
            marker = Path(td)/'runs.txt'
            with FailureApp(config(marker)) as app:
                result = app.cli('plugin', 'restart', 'probe', ok=False)
                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertEqual(len(marker.read_text().splitlines()), 1,
                                 'restart launched a new process after a failed stop')
                self.assertEqual(app.json('status')['plugin_processes'][0]['state'], 'stopped')

    def test_instance_stop_returns_cleanup_failure_and_removes_owned_state(self):
        with tempfile.TemporaryDirectory() as td:
            marker = Path(td)/'runs.txt'
            app = App(config(marker))
            try:
                app.start()
                result = app.cli('stop', ok=False)
                self.assertNotEqual(result.returncode, 0, result.stdout)
                eventually(lambda:not app.state.exists())
                # A cleanup failure must not leave a running replacement process.
                self.assertEqual(len(marker.read_text().splitlines()), 1)
                combined = result.stdout + result.stderr
                self.assertTrue('7' in combined or 'failed' in combined.lower(), combined)
            finally:
                if app.state.exists():
                    app.cli('stop', ok=False)
                app.temp.cleanup()

    def test_previous_terminal_report_cannot_prove_an_unregistered_restart_ready(self):
        with tempfile.TemporaryDirectory() as td:
            marker = Path(td)/'runs.txt'
            with App(config(marker, terminal_once=True)) as app:
                eventually(lambda:app.json('status')['plugin_processes'][0]['state']=='stopped')
                result = app.cli('plugin', 'restart', 'probe', ok=False)
                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertEqual(len(marker.read_text().splitlines()), 2)
                status = app.json('status')
                self.assertFalse(status['plugins'][0]['connected'])
                self.assertEqual(status['plugin_processes'][0]['state'], 'stopped')


if __name__ == '__main__':
    unittest.main(verbosity=2)
