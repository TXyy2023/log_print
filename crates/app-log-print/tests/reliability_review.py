#!/usr/bin/env python3
"""Independent reliability regression review against actual Core processes.
Run after building: python3 crates/app-log-print/tests/reliability_review.py
"""
import argparse
from contextlib import closing
import importlib.util
import json
from pathlib import Path
import socket
import sqlite3
import struct
import sys
import tempfile
import time
import traceback

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location('protocol_harness', ROOT / 'tests/protocol.py')
protocol = importlib.util.module_from_spec(spec)
spec.loader.exec_module(protocol)
Harness, RemoteError = protocol.Harness, protocol.RemoteError


def saved_fixture(directory, count=15):
    h = Harness(protocol.saved_config(directory), directory)
    h.start()
    with h.client('input') as source:
        for i in range(count):
            source.publish('saved', str(i), bytes([i]) * 16384)
    h.stop()
    return h


def assert_missing_is_blocked(h, before, description):
    h.start()
    with h.client() as admin, h.client('input') as source:
        status = next(s for s in admin.call('status')['streams'] if s['id'] == 'saved')
        assert status['blocked'], f'{description}: history silently accepted, acknowledged head={before}, reopened status={status}'
        try:
            source.publish('saved', 'after-loss', b'must not overwrite old sequence')
        except RemoteError as error:
            assert error.code in ('storage_blocked', 'history_unavailable'), error
        else:
            raise AssertionError('affected saved stream accepted publication despite missing history')
        assert source.publish('memory', 'unrelated', b'available')['durability'] == 'buffered'


def test_missing_tail_segment_blocks_saved_stream():
    with tempfile.TemporaryDirectory(prefix='log-print-review-tail-') as tmp:
        h = saved_fixture(Path(tmp))
        try:
            segments = sorted((Path(tmp) / 'data/saved').glob('*.sqlite'))
            assert len(segments) >= 2
            segments[-1].unlink()
            assert_missing_is_blocked(h, 15, 'removed final saved segment')
        finally:
            h.stop()
            h.log.close()


def test_missing_all_segments_is_not_new_history():
    with tempfile.TemporaryDirectory(prefix='log-print-review-all-') as tmp:
        h = saved_fixture(Path(tmp), 3)
        try:
            for segment in (Path(tmp) / 'data/saved').glob('*.sqlite'):
                segment.unlink()
            assert_missing_is_blocked(h, 3, 'removed all saved segments in existing store')
        finally:
            h.stop()
            h.log.close()


def test_interior_row_and_checksum_damage_are_reported():
    for kind in ('row', 'checksum'):
        with tempfile.TemporaryDirectory(prefix='log-print-review-damage-') as tmp:
            h = saved_fixture(Path(tmp), 3)
            try:
                path = sorted((Path(tmp) / 'data/saved').glob('*.sqlite'))[0]
                with closing(sqlite3.connect(path)) as db:
                    if kind == 'row':
                        db.execute('DELETE FROM records WHERE seq=2')
                    else:
                        db.execute("UPDATE records SET checksum='damaged' WHERE seq=2")
                    db.commit()
                h.start()
                with h.client() as admin:
                    try:
                        admin.call('read', stream='saved', **{'from': 1, 'limit': 64})
                    except RemoteError as error:
                        assert error.code in ('history_gap', 'history_unavailable', 'storage_blocked'), error
                    else:
                        raise AssertionError(f'{kind} damage was presented as valid complete history')
            finally:
                h.stop()
                h.log.close()


def test_processing_feedback_cannot_hide_behind_empty_parents():
    config = {'plugins': [
        {'id': 'a', 'bin': 'unused', 'reads': ['b'], 'streams': [{'id': 'a'}]},
        {'id': 'b', 'bin': 'unused', 'reads': ['a'], 'streams': [{'id': 'b'}]},
    ]}
    with tempfile.TemporaryDirectory(prefix='log-print-review-cycle-') as tmp:
        h = Harness(config, Path(tmp))
        try:
            try:
                h.start()
            except RuntimeError as error:
                message = str(error).lower()
                assert any(term in message for term in ('cycle', 'parent', 'dependency', 'feedback')), message
                return
            raise AssertionError('mutually consuming producers were accepted by declaring no parents')
        finally:
            h.stop()
            h.log.close()


def test_rst_during_hello_releases_plugin_identity():
    with Harness() as h, h.client() as admin:
        host, port = h.address.rsplit(':', 1)
        hello = (json.dumps({'protocol': 'log-print/1', 'plugin': 'input', 'token': 'input', 'events': False}) + '\n').encode()
        for _ in range(80):
            connection = socket.create_connection((host, int(port)), timeout=3)
            linger_format = 'HH' if sys.platform == 'win32' else 'ii'
            connection.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, struct.pack(linger_format, 1, 0))
            connection.sendall(hello)
            connection.close()
            time.sleep(.005)
        until = time.monotonic() + 3
        while time.monotonic() < until:
            connected = next(p for p in admin.call('status')['plugins'] if p['id'] == 'input')['connected']
            if not connected:
                break
            time.sleep(.03)
        assert not connected, 'RST hello left a stale plugin registry entry'
        with h.client('input') as fresh:
            assert fresh.publish('raw', 'reconnected', b'ready')['seq'] == 1


def test_idempotency_includes_source_timestamp():
    with Harness() as h, h.client('input') as source:
        fields = {'stream': 'raw', 'key': 'timestamp-key', 'payload': [1, 2], 'upstream': {}, 'source_ts_ns': 10}
        first = source.call('publish', **fields)
        assert source.call('publish', **fields) == first
        fields['source_ts_ns'] = 11
        protocol.expect('key_conflict', lambda: source.call('publish', **fields))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--case')
    parser.add_argument('--report', type=Path, default=ROOT / 'artifacts/reliability-review-results.json')
    args = parser.parse_args()
    results = []
    for name, fn in list(globals().items()):
        if not name.startswith('test_') or not callable(fn) or args.case and args.case not in name:
            continue
        before = time.monotonic()
        try:
            fn()
            row = {'case': name, 'status': 'pass'}
        except Exception:
            row = {'case': name, 'status': 'fail', 'error': traceback.format_exc()}
        row['seconds'] = round(time.monotonic() - before, 3)
        results.append(row)
        print(json.dumps(row, ensure_ascii=False), flush=True)
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps({'results': results}, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    return 1 if any(r['status'] == 'fail' for r in results) else 0


if __name__ == '__main__':
    raise SystemExit(main())
