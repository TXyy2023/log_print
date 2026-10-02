#!/usr/bin/env python3
"""Native CLI launch and readable output acceptance using real child processes."""
import json
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import unittest

from support import App, BIN, EXE, ROOT, eventually


class NativeApp(App):
    def __init__(self, *options):
        super().__init__({})
        self.config.unlink()  # This launch must work without a user configuration file.
        self.options = options

    def start(self):
        result = self.cli('start', *self.options)
        self.started = True
        assert result.stdout.startswith('Started.\n'), result.stdout
        return result


class NativeCliTests(unittest.TestCase):
    def readable(self, result):
        self.assertTrue(result.stdout.strip(), result)
        with self.assertRaises(json.JSONDecodeError):
            json.loads(result.stdout)
        return result.stdout

    def test_follow_file_lifecycle_and_named_controls(self):
        with tempfile.TemporaryDirectory() as td:
            source = Path(td)/'中文 file=a.log'
            source.write_text('已有内容\n', encoding='utf-8')
            with NativeApp('--input-file', f'source={source}', '--set', 'source.from_start=true',
                           '--output-raw', 'screen', '--no-autostart', 'screen',
                           '--describe', 'source=应用日志') as app:
                stream = app.inspect('streams')[0]['id']
                self.assertIn(stream, self.readable(app.cli('streams')))
                source.write_text('已有内容\n新增内容\n', encoding='utf-8')
                def payload():
                    return b''.join(bytes(r['payload']) for r in app.inspect('read', stream)['records'])
                eventually(lambda:b'\xe6\x96\xb0\xe5\xa2\x9e' in payload())
                data = self.readable(app.cli('read', stream))
                self.assertIn('已有内容', data)
                self.assertIn('新增内容', data)
                self.assertIn('source_seq:', data)
                self.assertIn('command line', self.readable(app.cli('config')))
                self.assertIn('from_start: yes', self.readable(app.cli('config', '--plugin', 'source')))
                self.assertIn(stream, self.readable(app.cli('resolve', 'source')))
                self.readable(app.cli('describe', stream, '编译输出'))
                self.assertEqual(app.inspect('stream', stream)['description'], '编译输出')
                self.readable(app.cli('call', 'stream.describe', f'stream={stream}', '--text', 'description=true'))
                self.assertEqual(app.inspect('stream', stream)['description'], 'true')
                self.readable(app.cli('plugin', 'call', 'source', 'config.get'))
                self.readable(app.cli('plugin', 'start', 'screen', '--stream', stream))
                self.assertIn('success: yes', self.readable(app.cli('plugin', 'stop', 'screen')))
                self.assertIn('started:', self.readable(app.cli('plugin', 'restart', 'screen')))
                self.assertIn('plugin_processes:', self.readable(app.cli('status')))
                stopped = self.readable(app.cli('stop'))
                self.assertIn('success: yes', stopped)
                self.assertIn('forced: no', stopped)
                self.assertFalse(app.state.exists())

    def test_spawn_args_binary_payload_and_raw_bytes(self):
        script = "import os,time,sys;assert sys.argv[1]=='001';os.write(1,b'out\\x00\\xff');os.write(2,os.environ['FLAG'].encode());time.sleep(1)"
        with NativeApp('--input-program', f'source={sys.executable}',
                       '--list-text', 'source.args=-u', '--list-text', 'source.args=-c',
                       '--list-text', f'source.args={script}', '--list-text', 'source.args=001', '--text', 'source.env.FLAG=false') as app:
            stream = app.inspect('streams')[0]['id']
            records = eventually(lambda:app.inspect('read', stream)['records'])
            eventually(lambda:{r['channel'] for r in app.inspect('read', stream)['records']} == {'stdout','stderr'})
            records = app.inspect('read', stream)['records']
            expected = b''.join(bytes(r['payload']) for r in records)
            result = subprocess.run([str(BIN/('log-print'+EXE)), '--state', str(app.state),
                                     'read', stream, '--raw'], cwd=ROOT, capture_output=True, timeout=5)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout, expected)
            rendered = self.readable(app.cli('read', stream))
            self.assertIn('payload (hex): 6f 75 74 00 ff', rendered)
            self.assertIn('payload: false', rendered)

    def test_transform_and_both_archive_shortcuts(self):
        with tempfile.TemporaryDirectory() as td:
            source = Path(td)/'source.log'
            archive = Path(td)/'capture.log'
            database = Path(td)/'capture.sqlite'
            source.write_bytes(b'fixture archive\n')
            with NativeApp('--input-file', f'source={source}', '--set', 'source.mode=static',
                           '--no-autostart', 'source', '--output-transform', 'derived',
                           '--set', 'derived.number=true', '--output-file', f'archive={archive}',
                           '--read', 'archive=derived', '--output-sqlite', f'database={database}',
                           '--read', 'database=derived') as app:
                self.readable(app.cli('plugin', 'start', 'source'))
                expected = b'[n=1] fixture archive\n'
                eventually(lambda:archive.exists() and archive.read_bytes() == expected)
                report = self.readable(app.cli('plugin', 'call', 'archive', 'status.get'))
                self.assertIn('confirmed:', report)
                stopped = self.readable(app.cli('stop'))
                self.assertIn('success: yes', stopped)
                self.assertEqual(archive.read_bytes(), expected)
                with sqlite3.connect(database) as connection:
                    payloads = connection.execute('SELECT payload FROM records').fetchall()
                self.assertEqual(b''.join(row[0] for row in payloads), expected)

    def test_multi_input_explicit_binding_and_empty_instance(self):
        with NativeApp() as app:
            self.assertEqual(app.cli('streams').stdout, 'No streams.\n')
            self.readable(app.cli('status'))
        with tempfile.TemporaryDirectory() as td:
            first = Path(td)/'first.log'
            second = Path(td)/'second.log'
            first.write_bytes(b'first')
            second.write_bytes(b'second')
            with NativeApp('--input-file', f'first={first}', '--input-file', f'second={second}',
                           '--set', 'first.mode=static', '--set', 'second.mode=static',
                           '--output-raw', 'screen', '--read', 'screen=second') as app:
                streams = app.inspect('streams')
                self.assertEqual({s['owner'] for s in streams}, {'first','second'})
                self.assertIn('first', self.readable(app.cli('streams')))
                eventually(lambda:b'second' in (app.path/'state.json.stdout.log').read_bytes())
                self.assertNotIn(b'first', (app.path/'state.json.stdout.log').read_bytes())

    def test_foreground_launch_stops_owned_children(self):
        with tempfile.TemporaryDirectory() as td:
            state = Path(td)/'foreground.json'
            with (Path(td)/'stdout.log').open('wb') as stdout, (Path(td)/'stderr.log').open('wb') as stderr:
                process = subprocess.Popen([str(BIN/('log-print'+EXE)), '--state', str(state), 'run'],
                                           cwd=ROOT, stdout=stdout, stderr=stderr)
                try:
                    eventually(lambda:state.exists() and 'address' in json.loads(state.read_text()))
                    result = subprocess.run([str(BIN/('log-print'+EXE)), '--state', str(state), 'stop'],
                                            cwd=ROOT, capture_output=True, text=True, timeout=15)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.readable(result)
                    self.assertEqual(process.wait(timeout=5), 0)
                    self.assertFalse(state.exists())
                finally:
                    if process.poll() is None:
                        process.terminate()
                        process.wait(timeout=8)


if __name__ == '__main__':
    unittest.main(verbosity=2)
