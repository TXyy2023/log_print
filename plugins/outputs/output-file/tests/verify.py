#!/usr/bin/env python3
"""Actual Core + output-file process acceptance; stdlib, bounded owned children.

Failure injection tests establish process-crash recovery, never power-loss safety.
All data, Core history, and diagnostic logs live below the chosen artifact folder.
"""
from __future__ import annotations

import argparse
import copy
from contextlib import closing
import csv
import importlib.util
import json
import os
from pathlib import Path
import platform
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
import traceback

ROOT = Path(__file__).resolve().parents[4]
BIN = ROOT / 'target' / os.environ.get('LOG_PRINT_PROFILE', 'debug')
EXE = '.exe' if os.name == 'nt' else ''
_spec = importlib.util.spec_from_file_location('archive_protocol_fixture', ROOT / 'tests/protocol.py')
protocol = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(protocol)


def wait(check, message, seconds=12):
    until = time.monotonic() + seconds
    while time.monotonic() < until:
        value = check()
        if value:
            return value
        time.sleep(.025)
    raise AssertionError(message)


def read_json(path):
    try:
        return json.loads(path.read_text(encoding='utf-8'))
    except (FileNotFoundError, json.JSONDecodeError):
        return None


class Archive:
    def __init__(self, directory, mode='both', *, fmt='raw', saved=True,
                 streams=('raw', 'second'), buffer_records=4096, extra=()):
        self.directory = directory
        directory.mkdir(parents=True, exist_ok=True)
        self.streams = list(streams)
        self.paths = {s: directory / f'{s}.{fmt}' for s in streams}
        self.db = directory / 'archive.sqlite'
        self.config = {'streams': list(streams), 'from': 1, 'mode': 'create'}
        if mode in ('file', 'both'):
            self.config['file'] = {'format': fmt, 'paths': {s: str(p) for s, p in self.paths.items()}}
        if mode in ('sqlite', 'both'):
            self.config['sqlite'] = {'path': str(self.db)}
        specs = [
            {'id': 'input', 'bin': 'unused', 'streams': [{'id': s, 'save': {'enabled': saved}} for s in streams]},
            {'id': 'archive', 'bin': 'output-file', 'reads': list(streams)},
            {'id': 'second_archive', 'bin': 'output-file', 'reads': list(streams)},
            *extra,
        ]
        config = {'core': {'buffer_records': buffer_records,
                         'save': {'directory': str(directory / 'core-data')}}, 'plugins': specs}
        self.h = protocol.Harness(config, directory)
        self.children = []
        self.logs = []
        self.process = None
        self.generation = 0

    def __enter__(self):
        self.h.start()
        return self

    def __exit__(self, *args):
        for process in self.children:
            if process.poll() is None:
                process.kill()
        for process in self.children:
            process.wait(timeout=5)
        self.h.__exit__(*args)
        for log in self.logs:
            log.close()

    def start(self, *, config=None, env=None, name='archive', binary='output-file', ready=True):
        effective = copy.deepcopy(config if config is not None else self.config)
        child_env = dict(os.environ, LOG_PRINT_CORE=self.h.address, LOG_PRINT_PLUGIN=name,
                         LOG_PRINT_TOKEN=name, LOG_PRINT_CONFIG=json.dumps(effective))
        child_env.update(env or {})
        self.generation += 1
        log = (self.directory / f'{name}-{self.generation}.stderr').open('wb')
        self.logs.append(log)
        process = subprocess.Popen([str(BIN / (binary + EXE))], env=child_env,
                                   cwd=self.directory, stdout=subprocess.DEVNULL, stderr=log)
        self.children.append(process)
        if name == 'archive':
            self.process = process
        if ready:
            def initialized():
                assert process.poll() is None, self.diagnostics()
                report = self.status(name)
                return report and report.get('state') in ('running', 'archiving', 'outputting', 'ready')
            wait(initialized, 'archive never became ready')
        return process

    def diagnostics(self):
        return '\n'.join(f'{p.name}: {p.read_text(errors="replace")}' for p in self.directory.glob('*.stderr'))

    def status(self, name='archive'):
        with self.h.client() as admin:
            return next(p.get('report') for p in admin.call('status')['plugins'] if p['id'] == name)

    def stop(self, name='archive'):
        with self.h.client() as admin:
            result = admin.call('control', target=name, method='shutdown')
        if name == 'archive':
            assert self.process.wait(timeout=8) == 0, self.diagnostics()
        return result

    def fail(self, process=None, seconds=12):
        process = process or self.process
        result = process.wait(timeout=seconds)
        assert result != 0, ('failure was reported as process success', self.diagnostics())
        return self.diagnostics()

    def publish(self, values, stream='raw'):
        with self.h.client('input') as source:
            for key, payload in enumerate(values):
                source.call('publish', stream=stream, key=f'{stream}-{time.monotonic_ns()}-{key}',
                            payload=list(payload), source_ts_ns=18446744073709551615)
        return self.records(stream)

    def records(self, stream='raw'):
        rows = []
        with self.h.client() as admin:
            next_seq = 1
            while True:
                page = admin.call('read', stream=stream, **{'from': next_seq})
                rows.extend(page['records'])
                next_seq = page['next']
                if next_seq > page['head']:
                    return rows

    def sql_records(self):
        if not self.db.exists():
            return []
        try:
            with closing(sqlite3.connect(self.db, timeout=.1)) as db:
                db.row_factory = sqlite3.Row
                records = []
                for row in db.execute('SELECT * FROM records'):
                    record = dict(row)
                    record.pop('record_sha256', None)
                    for key in ('seq', 'observed_ts_ns', 'source_ts_ns'):
                        record[key] = int(record[key]) if record[key] is not None else None
                    record['payload'] = list(record['payload'])
                    for key in ('upstream', 'upstream_epochs'):
                        record[key] = json.loads(record[key])
                    records.append(record)
                return records
        except sqlite3.OperationalError:
            return []

    def sql_count(self):
        if not self.db.exists():
            return 0
        try:
            with closing(sqlite3.connect(self.db, timeout=.1)) as db:
                return db.execute('SELECT COUNT(*) FROM records').fetchone()[0]
        except sqlite3.OperationalError:
            return 0

    def await_count(self, count, seconds=12):
        if 'sqlite' in self.config:
            wait(lambda: self.sql_count() == count, f'database did not reach {count}', seconds)
        else:
            def confirmed():
                total = 0
                for path in self.paths.values():
                    state = read_json(Path(str(path) + '.checkpoint.json'))
                    if state is None:
                        return False
                    total += int(state['cursor']['next']) - 1
                return total >= count
            wait(confirmed, f'files did not confirm {count}', seconds)

    def assert_records(self, expected):
        ordered = lambda rs: sorted(rs, key=lambda r: (r['stream'], r['seq']))
        if 'sqlite' in self.config:
            assert ordered(self.sql_records()) == ordered(expected), 'full SQLite Record mismatch'
            with closing(sqlite3.connect(self.db)) as db:
                assert db.execute('PRAGMA integrity_check').fetchone()[0] == 'ok'
                assert db.execute('PRAGMA journal_mode').fetchone()[0].lower() == 'wal'
                columns = ('stream', 'epoch', 'seq', 'key', 'payload', 'source_ts_ns',
                           'observed_ts_ns', 'upstream', 'upstream_epochs', 'durability')
                rows = db.execute('SELECT ' + ','.join(columns) + ' FROM records').fetchall()
                decoded = []
                for row in rows:
                    r = dict(zip(columns, row))
                    assert isinstance(r['seq'], str) and isinstance(r['observed_ts_ns'], str)
                    r['seq'] = int(r['seq'])
                    r['observed_ts_ns'] = int(r['observed_ts_ns'])
                    r['source_ts_ns'] = None if r['source_ts_ns'] is None else int(r['source_ts_ns'])
                    r['payload'] = list(r['payload'])
                    r['upstream'] = json.loads(r['upstream'])
                    r['upstream_epochs'] = json.loads(r['upstream_epochs'])
                    decoded.append(r)
                assert ordered(decoded) == ordered(expected), 'SQLite explicit columns differ from source'
        if 'file' in self.config:
            for stream, path in self.paths.items():
                records = [r for r in expected if r['stream'] == stream]
                if self.config['file']['format'] == 'raw':
                    assert path.read_bytes() == b''.join(bytes(r['payload']) for r in records), stream
                else:
                    lines = [json.loads(line) for line in path.read_text().splitlines()]
                    assert all(line['format_version'] == 1 for line in lines), lines[:1]
                    assert [line['record'] for line in lines] == records


def byte_modes(directory, mode, fmt='raw'):
    with Archive(directory, mode, fmt=fmt) as a:
        a.start()
        payloads = [b'A\x00\xffwithout-newline', b'', b'\xe4', b'\xb8\xad', bytes(range(256)) * 256]
        expected = a.publish(payloads) + a.publish([b'other\x00stream', b''], 'second')
        a.await_count(len(expected))
        with a.h.client() as admin:
            effective = admin.call('control', target='archive', method='config.get')
            assert effective['effective']['commit'] == {'max_records': 64, 'max_bytes': 4194304, 'max_delay_ms': 100}, effective
            try:
                admin.call('control', target='archive', method='config.patch', args={'from': 2})
            except protocol.RemoteError as error:
                assert 'restart' in str(error) or error.code == 'invalid_control', str(error)
            else:
                raise AssertionError('business config changed without restart')
        a.stop()
        a.assert_records(expected)
        a.config['mode'] = 'resume'
        a.start()
        more = a.publish([b'resumed'])
        expected = more + a.records('second')
        a.await_count(len(expected))
        a.stop()
        a.assert_records(expected)


def from_zero_and_empty(directory):
    with Archive(directory, streams=('raw',)) as a:
        a.publish([b'old-record'])
        a.config['from'] = 0
        a.start()
        a.stop()
        a.assert_records([])
        expected = a.publish([b'published-while-offline'])[1:]
        a.config['mode'] = 'resume'
        a.start()
        a.await_count(1)
        a.stop()
        a.assert_records(expected)
    with Archive(directory / 'empty', streams=('raw',)) as a:
        a.start()
        a.stop()
        a.config['mode'] = 'resume'
        a.start()
        expected = a.publish([b'first-after-resume'])
        a.await_count(1)
        a.stop()
        a.assert_records(expected)


def existing_and_lock(directory):
    with Archive(directory, streams=('raw',)) as a:
        a.start()
        competing = a.start(name='second_archive', ready=False)
        assert any(word in a.fail(competing).lower() for word in ('lock', 'exist', 'occupied'))
        expected = a.publish([b'owned'])
        a.await_count(1)
        a.stop()
        a.fail(a.start(ready=False))
        a.assert_records(expected)


def invalid_configs(directory):
    with Archive(directory, streams=('raw',)) as a:
        good = copy.deepcopy(a.config)
        configs = [
            {**good, 'typo': True},
            {'streams': ['raw']},
            {**good, 'streams': []},
            {**good, 'streams': ['raw', 'raw']},
            {**good, 'streams': ['undeclared']},
            {**good, 'commit': {'max_records': 0}},
            {**good, 'commit': {'max_bytes': 0}},
            {**good, 'queue': {'max_bytes': 0}},
            {**good, 'sqlite': {'path': str(a.paths['raw'])}},
            {**good, 'file': {'format': 'unknown', 'paths': {'raw': str(a.paths['raw'])}}},
        ]
        for cfg in configs:
            a.fail(a.start(config=cfg, ready=False))
        assert not a.db.exists() and not a.paths['raw'].exists(), 'invalid config mutated a destination'


def external_changes(directory, change):
    with Archive(directory, streams=('raw',)) as a:
        a.start()
        a.publish([b'abcdefgh'])
        a.await_count(1)
        a.stop()
        path = a.paths['raw']
        if change == 'prefix':
            path.write_bytes(b'Abcdefgh')
        elif change == 'short':
            path.write_bytes(b'a')
        elif change == 'identity':
            replacement = directory / 'replacement'
            replacement.write_bytes(path.read_bytes())
            os.replace(replacement, path)
        elif change == 'checkpoint':
            Path(str(path) + '.checkpoint.json').write_text('{broken')
        elif change in ('database', 'database_delete', 'database_payload'):
            with closing(sqlite3.connect(a.db)) as db:
                # Make damage deterministic: a surviving WAL must not validly
                # restore the original page after main-file corruption.
                db.execute('PRAGMA wal_checkpoint(TRUNCATE)')
                if change == 'database_delete':
                    db.execute('DELETE FROM records')
                    db.commit()
                elif change == 'database_payload':
                    db.execute('UPDATE records SET payload=?', (b'changed!',))
                    db.commit()
                if change != 'database':
                    assert db.execute('PRAGMA integrity_check').fetchone()[0] == 'ok'
            if change == 'database':
                a.db.write_bytes(b'not a sqlite database')
        a.config['mode'] = 'resume'
        a.fail(a.start(ready=False))


def missing_history(directory, allow):
    with Archive(directory, streams=('raw',), saved=False, buffer_records=2) as a:
        a.start()
        a.stop()
        a.publish([b'a', b'b', b'c', b'd', b'e'])
        a.config.update(mode='resume', fail_on_gap=not allow)
        a.start(ready=False)
        if allow:
            a.await_count(2)
            a.stop()
            a.assert_records(a.records())
            checkpoint = read_json(Path(str(a.paths['raw']) + '.checkpoint.json'))
            assert checkpoint['incomplete'] is True and checkpoint['gap_count'] > 0, checkpoint
            with closing(sqlite3.connect(a.db)) as db:
                assert db.execute('SELECT COUNT(*) FROM gaps').fetchone()[0] > 0
        else:
            a.fail()
            assert not a.sql_records(), 'stop-on-gap consumed later records'


def epoch_change(directory):
    with Archive(directory, streams=('raw',), saved=False) as a:
        a.start()
        a.publish([b'old-epoch'])
        a.await_count(1)
        a.stop()
        a.h.stop()
        a.h.start()
        a.config['mode'] = 'resume'
        assert 'epoch' in a.fail(a.start(ready=False)).lower()


def failpoint_resume(directory, point):
    with Archive(directory, streams=('raw',)) as a:
        a.start()
        a.stop()
        expected = a.publish([b'one', b'', b'\x00\xfftwo'])
        a.config.update(mode='resume', commit={'max_records': 1})
        a.start(ready=False, env={'LOG_PRINT_ARCHIVE_TESTING': '1', 'LOG_PRINT_ARCHIVE_FAILPOINT': point})
        a.fail()
        a.start()
        a.await_count(len(expected))
        a.stop()
        a.assert_records(expected)


def force_kill(directory):
    with Archive(directory, streams=('raw',)) as a:
        a.start(env={'LOG_PRINT_ARCHIVE_TESTING': '1', 'LOG_PRINT_ARCHIVE_TEST_DELAY_MS': '100'})
        expected = a.publish([bytes([i]) * 512 for i in range(20)])
        wait(lambda: (a.status() or {}).get('received_records', 0) >= 1, 'did not receive before kill')
        a.process.kill()
        a.process.wait(timeout=5)
        a.config['mode'] = 'resume'
        a.start()
        a.await_count(len(expected))
        a.stop()
        a.assert_records(expected)


def disconnected(directory):
    with Archive(directory, streams=('raw',)) as a:
        a.start()
        a.publish([b'before-disconnect'])
        a.await_count(1)
        a.h.stop()
        error = a.fail().lower()
        assert any(word in error for word in ('disconnect', 'connection', 'closed')), error


def partial_initialization(directory):
    with Archive(directory, streams=('raw',)) as a:
        # Two new targets cannot pretend to form a resumable archive after a
        # failure opening the second target. A user-owned blocker is preserved.
        a.db.write_bytes(b'user-owned')
        a.fail(a.start(ready=False))
        assert a.db.read_bytes() == b'user-owned'
        a.config['mode'] = 'resume'
        a.fail(a.start(ready=False))
        assert a.db.read_bytes() == b'user-owned'


def permission_failure(directory):
    if os.name == 'nt' or (hasattr(os, 'geteuid') and os.geteuid() == 0):
        return {'status': 'skipped', 'reason': 'requires unprivileged POSIX chmod semantics'}
    with Archive(directory, 'file', streams=('raw',)) as a:
        a.start()
        a.stop()
        path = a.paths['raw']
        path.chmod(0o400)
        try:
            a.config['mode'] = 'resume'
            a.fail(a.start(ready=False))
        finally:
            path.chmod(0o600)


def batch_clock(directory):
    with Archive(directory, streams=('raw',)) as a:
        a.start()
        started = time.monotonic()
        a.publish([b'low-traffic'])
        a.await_count(1, seconds=3)
        low_seconds = time.monotonic() - started
        # Do not assert 100 ms as a scheduling/latency promise; require the first
        # batch to persist while records continue arriving below the count cap.
        with a.h.client('input') as source:
            started = time.monotonic()
            while time.monotonic() - started < .7:
                source.publish('raw', f'continuous-{time.monotonic_ns()}', b'x')
                time.sleep(.01)
            assert len(a.sql_records()) > 1, 'continuous input postponed timed commit indefinitely'
        a.stop()
        a.assert_records(a.records())
        return {'low_traffic_commit_seconds': round(low_seconds, 3)}


def backpressure_shutdown(directory):
    with Archive(directory, streams=('raw',)) as a:
        a.config['queue'] = {'max_records': 4, 'max_bytes': 1048576}
        a.config['commit'] = {'max_records': 1}
        a.start(env={'LOG_PRINT_ARCHIVE_TESTING': '1', 'LOG_PRINT_ARCHIVE_TEST_DELAY_MS': '120'})
        a.publish([bytes([i]) * 65536 for i in range(24)])
        observed = []
        until = time.monotonic() + 1.2
        while time.monotonic() < until:
            report = a.status()
            queue = report['queue']
            assert queue['records'] <= 4 and queue['bytes'] <= 1048576, queue
            observed.append(queue['records'])
            time.sleep(.025)
        assert max(observed) > 0, 'slow writer failed to exercise queued backpressure'
        received = a.status()['received_records']
        a.stop()
        records = a.sql_records()
        assert len(records) >= received, ('shutdown acknowledged before received records persisted', received, len(records))
        a.assert_records(records)
        a.config['mode'] = 'resume'
        a.start()
        a.await_count(24)
        a.stop()
        a.assert_records(a.records())
        return {'maximum_reported_queue_records': max(observed), 'received_before_shutdown': received}


def control_timeout(directory):
    with Archive(directory, streams=('raw',)) as a:
        a.config['commit'] = {'max_records': 1}
        a.start(env={'LOG_PRINT_ARCHIVE_TESTING': '1', 'LOG_PRINT_ARCHIVE_TEST_DELAY_MS': '11500'})
        expected = a.publish([b'pending-at-timeout'])
        wait(lambda: (a.status() or {}).get('received_records', 0) == 1, 'delayed record not received')
        with a.h.client() as admin:
            admin.socket.settimeout(14)
            try:
                admin.call('control', target='archive', method='shutdown')
            except protocol.RemoteError as error:
                assert 'timeout' in str(error).lower(), str(error)
            else:
                raise AssertionError('slow shutdown returned success within Core timeout')
        assert a.process.wait(timeout=8) == 0, a.diagnostics()
        a.assert_records(expected)



def actual_replay_transform(directory):
    a = Archive(directory, fmt='jsonl', streams=('raw', 'derived'))
    source = directory / 'input.bin'
    payload = 'temp=中\nnext=2'.encode()
    source.write_bytes(payload)
    source_config = {'path': str(source), 'stream': 'raw', 'timestamp': 'none', 'interval_ms': 0, 'chunk_bytes': 1}
    transform_config = {'streams': ['raw'], 'output_stream': 'derived', 'input_encoding': 'utf-8',
                        'split_lines': True, 'replace': [{'pattern': 'temp=', 'with': 'T='}]}
    specs = [
        {'id': 'source', 'bin': 'input-replay', 'streams': [{'id': 'raw', 'save': {'enabled': True}}]},
        {'id': 'transform', 'bin': 'output-transform', 'reads': ['raw'],
         'streams': [{'id': 'derived', 'parents': ['raw'], 'save': {'enabled': True}}]},
        {'id': 'archive', 'bin': 'output-file', 'reads': ['raw', 'derived']},
    ]
    a.h.runtime['config']['plugins'] = specs
    a.h.runtime['plugin_tokens'] = {p['id']: p['id'] for p in specs}
    a.h.path.write_text(json.dumps(a.h.runtime))
    with a:
        a.start()
        transform = a.start(name='transform', binary='output-transform', config=transform_config, ready=False)
        source_process = a.start(name='source', binary='input-replay', config=source_config, ready=False)
        assert source_process.wait(timeout=8) == 0, a.diagnostics()
        wait(lambda: len(a.records('derived')) == 1, 'transform line not produced')
        a.stop('transform')
        assert transform.wait(timeout=8) == 0, a.diagnostics()
        expected = a.records('raw') + a.records('derived')
        a.await_count(len(expected))
        a.stop()
        a.assert_records(expected)
        derived = a.records('derived')
        assert b''.join(bytes(r['payload']) for r in derived) == 'T=中\nnext=2'.encode()
        assert all(r['upstream'] and r['upstream_epochs'] for r in derived)


def checkpoint_write_failure(directory):
    if os.name == 'nt' or (hasattr(os, 'geteuid') and os.geteuid() == 0):
        return {'status': 'skipped', 'reason': 'requires unprivileged POSIX chmod semantics'}
    with Archive(directory, streams=('raw',)) as a:
        a.config['commit'] = {'max_records': 1}
        a.start()
        directory.chmod(0o500)
        try:
            expected = a.publish([b'unconfirmed-when-checkpoint-is-denied'])
            error = a.fail().lower()
            assert any(word in error for word in ('permission', 'denied', 'checkpoint')), error
        finally:
            directory.chmod(0o700)
        a.config['mode'] = 'resume'
        a.start()
        a.await_count(1)
        a.stop()
        a.assert_records(expected)


def process_rss_kib(pid):
    if os.name == 'nt':
        text = subprocess.check_output(['tasklist', '/FI', f'PID eq {pid}', '/FO', 'CSV', '/NH'], text=True)
        row = next(csv.reader(text.splitlines()))
        return int(''.join(c for c in row[-1] if c.isdigit()))
    return int(subprocess.check_output(['ps', '-o', 'rss=', '-p', str(pid)], text=True).strip())


def sustained_memory(directory):
    with Archive(directory, streams=('raw',)) as a:
        a.config['queue'] = {'max_records': 4, 'max_bytes': 1048576}
        a.start(env={'LOG_PRINT_ARCHIVE_TESTING': '1', 'LOG_PRINT_ARCHIVE_TEST_DELAY_MS': '5'})
        errors = []
        count, size = 512, 32768
        def publish():
            try:
                with a.h.client('input') as source:
                    for i in range(count):
                        source.publish('raw', f'load-{i}', bytes([i % 256]) * size)
            except BaseException as error:
                errors.append(repr(error))
        publisher = threading.Thread(target=publish, daemon=True)
        publisher.start()
        samples = []
        started = time.monotonic()
        try:
            while publisher.is_alive() or a.sql_count() < count:
                assert time.monotonic() - started < 40, 'sustained load did not finish'
                assert a.process.poll() is None, a.diagnostics()
                report = a.status()
                queue = report['queue']
                assert queue['records'] <= 4 and queue['bytes'] <= 1048576, report
                samples.append(process_rss_kib(a.process.pid))
                time.sleep(.05)
            publisher.join(timeout=5)
            assert not publisher.is_alive() and not errors, errors
            assert samples and max(samples) < 256 * 1024, ('archive RSS exceeded 256 MiB acceptance bound', samples)
            a.stop()
            # Validate count and a streaming aggregate without loading all 16 MiB
            # of source records into the supervising test process.
            with closing(sqlite3.connect(a.db)) as db:
                assert db.execute('SELECT COUNT(*), SUM(LENGTH(payload)) FROM records').fetchone() == (count, count * size)
            raw = a.paths['raw'].read_bytes()
            assert raw == b''.join(bytes([i % 256]) * size for i in range(count))
        finally:
            publisher.join(timeout=12)
        return {'records': count, 'payload_bytes': count * size, 'peak_observed_rss_kib': max(samples),
                'rss_bound_kib': 256 * 1024, 'queue_records_limit': 4, 'queue_bytes_limit': 1048576}


def interrupted_initialization(directory):
    with Archive(directory, streams=('raw',)) as a:
        a.config['from'] = 0
        a.publish([b'old'])
        a.start(ready=False, env={'LOG_PRINT_ARCHIVE_TESTING': '1',
                                'LOG_PRINT_ARCHIVE_FAILPOINT': 'init_after_first_target'})
        a.fail()
        checkpoint = read_json(Path(str(a.paths['raw']) + '.checkpoint.json'))
        assert checkpoint['initial']['next'] == 2, checkpoint
        assert not a.db.exists(), 'initialization interruption did not precede second target'
        a.publish([b'must-not-be-skipped-by-reselecting-live-start'])
        a.config['mode'] = 'resume'
        a.fail(a.start(ready=False))
        assert read_json(Path(str(a.paths['raw']) + '.checkpoint.json')) == checkpoint
        assert not a.db.exists(), 'failed resume silently initialized a missing target'


def single_record_over_batch_budget(directory):
    with Archive(directory, streams=('raw',)) as a:
        # A legal maximum-payload Record exceeds this batch target, but must be
        # committed on its own rather than rejected or held indefinitely.
        a.config['commit'] = {'max_bytes': 32}
        a.start()
        expected = a.publish([bytes(range(256)) * 256, b''])
        a.await_count(len(expected))
        a.stop()
        a.assert_records(expected)


def injected_io_failure(directory, point):
    with Archive(directory, streams=('raw',)) as a:
        a.start()
        a.stop()
        expected = a.publish([b'first', b'second\x00\xff'])
        a.config.update(mode='resume', commit={'max_records': 1})
        a.start(ready=False, env={'LOG_PRINT_ARCHIVE_TESTING': '1', 'LOG_PRINT_ARCHIVE_ERRORPOINT': point})
        error = a.fail().lower()
        assert 'injected' in error, error
        report = a.status()
        assert report['state'] == 'failed' and report['complete'] is False, report
        a.start()
        a.await_count(len(expected))
        a.stop()
        a.assert_records(expected)
        return {'boundary': 'Injected I/O error return; not a physical disk-full experiment.'}


def many_streams_bounded_report(directory):
    streams = tuple(f'stream-{i:03}-status-budget-validation-padding' for i in range(128))
    with Archive(directory, 'sqlite', streams=streams, saved=False) as a:
        a.start()
        report = a.status()
        assert report['report_truncated'] is True, report
        assert report['total_streams'] == 128 and report['stream_details_control'] == 'status.get', report
        report_bytes = len(json.dumps(report, separators=(',', ':')).encode())
        assert report_bytes <= 16384, report_bytes
        with a.h.client() as admin:
            complete = admin.call('control', target='archive', method='status.get')
        assert set(complete['common']) == set(streams), complete
        assert all(cursor['next'] == 1 for cursor in complete['common'].values())
        a.stop()
        a.assert_records([])
        return {'streams': 128, 'serialized_report_bytes': report_bytes}

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--report', type=Path)
    parser.add_argument('--case', action='append', help='Run only exact case names (iteration only)')
    args = parser.parse_args()
    cases = [(f'{mode}_{fmt}_bytes_resume', lambda d, m=mode, f=fmt: byte_modes(d, m, f))
             for mode, fmt in [('file', 'raw'), ('sqlite', 'raw'), ('both', 'raw'), ('both', 'jsonl')]]
    cases += [('from_zero_empty_resume', from_zero_and_empty), ('existing_and_exclusive_lock', existing_and_lock),
              ('strict_config', invalid_configs), ('gap_stop', lambda d: missing_history(d, False)),
              ('gap_continue_persisted', lambda d: missing_history(d, True)), ('epoch_change', epoch_change),
              ('partial_initialization', partial_initialization), ('permission_failure', permission_failure),
              ('batch_clock', batch_clock), ('bounded_backpressure_shutdown', backpressure_shutdown),
              ('control_timeout_unknown_then_complete', control_timeout), ('forced_exit_resume', force_kill),
              ('disconnected_not_success', disconnected), ('real_replay_transform_records', actual_replay_transform),
              ('checkpoint_write_failure_resume', checkpoint_write_failure), ('sustained_load_memory', sustained_memory),
              ('interrupted_initialization', interrupted_initialization),
              ('single_record_over_batch_budget', single_record_over_batch_budget),
              ('many_streams_bounded_report', many_streams_bounded_report)]
    cases += [(f'external_{change}', lambda d, c=change: external_changes(d, c))
              for change in ('prefix', 'short', 'identity', 'checkpoint', 'database', 'database_delete', 'database_payload')]
    cases += [(f'crash_{point}', lambda d, p=point: failpoint_resume(d, p)) for point in
              ('file_after_write', 'file_after_sync', 'file_after_checkpoint',
               'sqlite_before_commit', 'sqlite_after_commit', 'between_targets')]
    cases += [(f'io_error_{point}', lambda d, p=point: injected_io_failure(d, p)) for point in
              ('file_write', 'file_sync', 'checkpoint_replace', 'sqlite_commit')]
    if args.case:
        names = {name for name, _ in cases}
        assert set(args.case) <= names, f'unknown cases: {set(args.case) - names}'
        cases = [(name, case) for name, case in cases if name in args.case]
    report = {'platform': platform.platform(), 'python': sys.version, 'iteration_only': bool(args.case),
              'boundary': 'Real subprocess termination and OS sync APIs; no physical power-loss or device tests.', 'cases': []}
    with tempfile.TemporaryDirectory(prefix='log-print-archive-') as tmp:
        work = Path(tmp) if args.report is None else args.report.resolve().parent / 'archive-artifacts'
        work.mkdir(parents=True, exist_ok=True)
        for name, case in cases:
            directory = work / name
            before = time.monotonic()
            print(f'RUN archive/{name}', flush=True)
            row = {'case': name, 'status': 'passed'}
            try:
                row.update(case(directory) or {})
            except Exception:
                row.update(status='failed', error=traceback.format_exc())
                print(row['error'], flush=True)
                for log in directory.glob('*.stderr'):
                    print(log.name, log.read_text(errors='replace')[-5000:], flush=True)
            row['seconds'] = round(time.monotonic() - before, 3)
            report['cases'].append(row)
            print(f'{row["status"].upper()} archive/{name}: {row["seconds"]}s', flush=True)
            if args.report:
                args.report.parent.mkdir(parents=True, exist_ok=True)
                args.report.write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
        report['passed'] = all(row['status'] in ('passed', 'skipped') for row in report['cases'])
        if args.report:
            args.report.write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
        print(json.dumps(report, indent=2), flush=True)
        return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
