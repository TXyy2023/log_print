#!/usr/bin/env python3
"""Real Core subprocess acceptance, no in-process mocks; Python 3.12+ stdlib."""
import argparse
import contextlib
import hashlib
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import threading
import time
import traceback

ROOT = Path(__file__).resolve().parents[1]
BIN = ROOT / "target" / os.environ.get("LOG_PRINT_PROFILE", "debug")
EXE = ".exe" if os.name == "nt" else ""
RESULTS = []


class RemoteError(Exception):
    def __init__(self, error):
        self.code = error["code"]
        super().__init__(f'{self.code}: {error["message"]}')


class RPC:
    def __init__(self, address, plugin="__admin__", token="admin", events=False, protocol="log-print/1"):
        host, port = address.rsplit(":", 1)
        self.socket = socket.create_connection((host, int(port)), timeout=10)
        self.socket.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        self.file = self.socket.makefile("rwb", buffering=0)
        self.sequence = 0
        self.send({"protocol": protocol, "plugin": plugin, "token": token, "events": events})
        self.result(self.receive())

    def send(self, value):
        self.socket.sendall(json.dumps(value, separators=(",", ":")).encode() + b"\n")

    def receive(self):
        line = self.file.readline(1024 * 1024 + 1)
        if not line:
            raise EOFError("Core disconnected")
        assert len(line) <= 1024 * 1024, "oversized server frame"
        return json.loads(line)

    @staticmethod
    def result(message):
        if message.get("error"):
            raise RemoteError(message["error"])
        return message.get("result")

    def call(self, op, **args):
        self.sequence += 1
        self.send({"id": self.sequence, "op": op, "args": args})
        result = self.receive()
        assert result["type"] == "response" and result["id"] == self.sequence, result
        return self.result(result)

    def publish(self, stream, key, data, upstream=None):
        return self.call("publish", stream=stream, key=key, payload=list(data), upstream=upstream or {})

    def close(self):
        self.file.close()
        self.socket.close()

    def __enter__(self):
        return self

    def __exit__(self, *args):
        self.close()


class Harness:
    def __init__(self, config=None, directory=None):
        self.temporary = tempfile.TemporaryDirectory(prefix="log-print-protocol-") if directory is None else None
        self.directory = Path(directory or self.temporary.name)
        self.config = config or {"core": {"buffer_records": 4}, "plugins": [
            {"id": "input", "bin": "unused", "streams": [{"id": "raw"}, {"id": "second"}]},
            {"id": "transform", "bin": "unused", "reads": ["raw"], "streams": [{"id": "derived", "parents": ["raw"]}]},
            {"id": "output", "bin": "unused", "reads": ["raw", "second", "derived"]},
        ]}
        self.runtime = {"config": self.config, "admin_token": "admin", "plugin_tokens": {p["id"]: p["id"] for p in self.config["plugins"]}}
        self.path = self.directory / "config.json"
        self.path.write_text(json.dumps(self.runtime))
        self.ready = self.directory / "ready.json"
        self.log = open(self.directory / "core.log", "ab")
        self.process = None

    def start(self):
        self.ready.unlink(missing_ok=True)
        self.process = subprocess.Popen([str(BIN / ("log-print-core" + EXE)), "--runtime-config", str(self.path), "--ready-file", str(self.ready)], stdin=subprocess.PIPE, stdout=self.log, stderr=self.log)
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            if self.process.poll() is not None:
                raise RuntimeError((self.directory / "core.log").read_text())
            try:
                self.address = json.loads(self.ready.read_text())["address"]
                return self
            except (FileNotFoundError, json.JSONDecodeError):
                time.sleep(0.02)
        raise TimeoutError("Core readiness")

    def client(self, plugin="__admin__", **kwargs):
        return RPC(self.address, plugin, "admin" if plugin == "__admin__" else plugin, **kwargs)

    def stop(self):
        if self.process and self.process.poll() is None:
            self.process.stdin.close()
            try:
                assert self.process.wait(timeout=10) == 0
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=3)
                raise AssertionError("Core did not stop after parent stdin EOF")

    def __enter__(self):
        return self.start()

    def __exit__(self, *args):
        self.stop()
        self.log.close()
        if self.temporary:
            self.temporary.cleanup()


def expect(code, fn):
    try:
        fn()
    except RemoteError as e:
        assert e.code == code, str(e)
    else:
        raise AssertionError(f"expected {code}")


def test_bytes_permissions_epochs():
    with Harness() as h, h.client("input") as source, h.client() as admin:
        data = bytes(range(256)) + b"without-newline\x00\xff"
        a = source.publish("raw", "first", data)
        assert a["seq"] == 1 and a["durability"] == "buffered"
        assert source.publish("raw", "first", data) == a
        expect("key_conflict", lambda: source.publish("raw", "first", b"changed"))
        assert bytes(admin.call("read", stream="raw")["records"][0]["payload"]) == data
        expect("permission_denied", lambda: source.call("read", stream="raw"))
        expect("permission_denied", lambda: admin.publish("raw", "foreign", b"x"))
        expect("epoch_mismatch", lambda: admin.call("read", stream="raw", epoch="old"))
        expect("limit", lambda: admin.call("read", stream="raw", limit=100))
        expect("limit", lambda: source.publish("raw", "big", bytes(65537)))
        expect("version_mismatch", lambda: RPC(h.address, protocol="log-print/0"))
        expect("authentication_failed", lambda: RPC(h.address, token="wrong"))


def test_multistream_and_derived():
    with Harness() as h, h.client("input") as source, h.client("transform") as transform, h.client() as admin:
        source.publish("raw", "a", b"raw")
        source.publish("second", "b", b"separate")
        expect("invalid_parents", lambda: transform.publish("derived", "d", b"derived"))
        expect("invalid_parent_cursor", lambda: transform.publish("derived", "d", b"derived", {"raw": 2}))
        transform.publish("derived", "d", b"derived", {"raw": 1})
        assert bytes(admin.call("read", stream="raw")["records"][0]["payload"]) == b"raw"
        assert bytes(admin.call("read", stream="derived")["records"][0]["payload"]) == b"derived"
        assert admin.call("read", stream="second")["records"][0]["seq"] == 1


def test_ring_and_live_only():
    with Harness() as h, h.client("input") as source, h.client() as admin:
        for i in range(10):
            source.publish("raw", str(i), str(i).encode())
        page = admin.call("read", stream="raw", **{"from": 1})
        assert page["gap"] == {"from": 1, "to": 6, "reason": "buffer_overwritten"}
        assert [r["seq"] for r in page["records"]] == [7, 8, 9, 10]
        with h.client("output", events=True) as live:
            live.call("subscribe", stream="raw", **{"from": 0})
            source.publish("raw", "11", b"new")
            assert live.receive()["record"]["payload"] == list(b"new")


def test_history_live_race():
    config = {"core": {"buffer_records": 4096}, "plugins": [{"id": "input", "bin": "unused", "streams": [{"id": "raw"}]}, {"id": "output", "bin": "unused", "reads": ["raw"]}]}
    with Harness(config) as h, h.client("input") as source, h.client("output", events=True) as output:
        for i in range(40):
            source.publish("raw", str(i), f"{i}\n".encode())
        error = []
        def publish():
            try:
                for i in range(40, 200):
                    source.publish("raw", str(i), f"{i}\n".encode())
            except BaseException as e:
                error.append(e)
        task = threading.Thread(target=publish)
        task.start()
        output.call("subscribe", stream="raw", **{"from": 1})
        records = [output.receive()["record"] for _ in range(200)]
        task.join(10)
        assert not task.is_alive() and not error, error
        assert [r["seq"] for r in records] == list(range(1, 201))
        assert b"".join(bytes(r["payload"]) for r in records) == b"".join(f"{i}\n".encode() for i in range(200))


def test_slow_output_isolation():
    with Harness() as h, h.client("input") as source, h.client("output", events=True) as slow, h.client() as admin:
        slow.socket.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, 4096)
        slow.call("subscribe", stream="raw", **{"from": 1})
        started = time.monotonic()
        for i in range(80):
            source.publish("raw", str(i), bytes(32768))
        source.publish("second", "free", b"independent")
        assert bytes(admin.call("read", stream="second")["records"][0]["payload"]) == b"independent"
        assert time.monotonic() - started < 20, "slow event socket blocked independent RPC"


def saved_config(directory):
    return {"core": {"buffer_records": 2, "save": {"directory": str(directory / "data"), "file_bytes": 262144, "total_bytes": 2 * 1024 * 1024}}, "plugins": [
        {"id": "input", "bin": "unused", "streams": [{"id": "saved", "save": {"enabled": True}}, {"id": "other", "save": {"enabled": True}}, {"id": "memory"}]},
        {"id": "output", "bin": "unused", "reads": ["saved", "other", "memory"]}]}


def test_saved_restart_rotation_idempotency():
    with tempfile.TemporaryDirectory(prefix="log-print-saved-") as tmp:
        directory = Path(tmp)
        with Harness(saved_config(directory), directory) as h:
            with h.client("input") as source, h.client() as admin:
                for i in range(15):
                    source.publish("saved", str(i), bytes([i]) * 16384)
                before = admin.call("read", stream="saved")
                assert not before["gap"]
                while before["next"] <= before["head"]:
                    page = admin.call("read", stream="saved", **{"from": before["next"]})
                    before["records"].extend(page["records"])
                    before["next"] = page["next"]
                assert len(before["records"]) == 15
                epoch = before["epoch"]
                assert len(list((directory / "data/saved").glob("*.sqlite"))) >= 2
                source.publish("memory", "m", b"ephemeral")
                memory_epoch = admin.call("read", stream="memory")["epoch"]
            h.stop()
            h.start()
            with h.client("input") as source, h.client() as admin:
                after = admin.call("read", stream="saved", epoch=epoch)
                while after["next"] <= after["head"]:
                    page = admin.call("read", stream="saved", epoch=epoch, **{"from": after["next"]})
                    after["records"].extend(page["records"])
                    after["next"] = page["next"]
                assert after["records"] == before["records"]
                retry = source.publish("saved", "0", bytes([0]) * 16384)
                assert retry["seq"] == 1
                assert admin.call("read", stream="saved")["head"] == 15
                expect("epoch_mismatch", lambda: admin.call("read", stream="memory", epoch=memory_epoch))
                assert admin.call("read", stream="memory")["head"] == 0


def test_capacity_failure_isolation():
    with tempfile.TemporaryDirectory(prefix="log-print-cap-") as tmp:
        d = Path(tmp)
        config = saved_config(d)
        config["plugins"][0]["streams"][0]["save"]["total_bytes"] = 524288
        with Harness(config, d) as h, h.client("input") as source, h.client() as admin:
            accepted = 0
            for i in range(100):
                try:
                    source.publish("saved", str(i), bytes([i]) * 32768)
                    accepted += 1
                except RemoteError as e:
                    assert e.code == "storage_blocked", e
                    break
            else:
                raise AssertionError("capacity limit was not enforced")
            source.publish("other", "ok", b"still saved")
            source.publish("memory", "ok", b"still buffered")
            assert admin.call("read", stream="other")["records"][0]["durability"] == "saved"
            assert admin.call("read", stream="memory")["records"][0]["durability"] == "buffered"
            assert admin.call("read", stream="saved")["head"] == accepted
            assert next(s for s in admin.call("status")["streams"] if s["id"] == "saved")["blocked"]


def test_hot_journal_process_recovery():
    with tempfile.TemporaryDirectory(prefix="log-print-journal-") as tmp:
        d = Path(tmp)
        with Harness(saved_config(d), d) as h:
            with h.client("input") as source:
                source.publish("saved", "committed", b"durable bytes")
            h.stop()
            database = sorted((d / "data/saved").glob("*.sqlite"))[-1]
            script = """import os,sqlite3,sys
c=sqlite3.connect(sys.argv[1])
c.execute('PRAGMA cache_size=1')
c.execute('BEGIN IMMEDIATE')
c.execute(\"UPDATE meta SET value='999' WHERE key='head'\")
for n in range(50):
 c.execute('INSERT INTO records VALUES(?,?,?,?)',(n+2,'uncommitted-'+str(n),'x'*2048,'invalid'))
os._exit(0)
"""
            subprocess.run([os.sys.executable, "-c", script, str(database)], check=True)
            assert database.with_name(database.name + "-journal").exists(), "fault injector did not leave a hot journal"
            h.start()
            with h.client() as admin, h.client("input") as source:
                page = admin.call("read", stream="saved")
                assert page["head"] == 1 and bytes(page["records"][0]["payload"]) == b"durable bytes"
                assert source.publish("saved", "after-recovery", b"next")["seq"] == 2


def test_unwritable_manual_resume():
    if os.name == "nt":
        return {"skip": "POSIX permission injection; quota failure tested on every platform"}
    with tempfile.TemporaryDirectory(prefix="log-print-permission-") as tmp:
        d = Path(tmp)
        with Harness(saved_config(d), d) as h, h.client("input") as source, h.client() as admin:
            source.publish("saved", "first", b"durable")
            store = d / "data/saved"
            store.chmod(0o500)
            try:
                expect("storage_blocked", lambda: source.publish("saved", "retry", b"retained"))
            finally:
                store.chmod(0o700)
            expect("storage_blocked", lambda: source.publish("saved", "retry", b"retained"))
            source.publish("other", "isolated", b"ok")
            admin.call("resume", stream="saved")
            assert source.publish("saved", "retry", b"retained")["seq"] == 2


def test_oversized_wire():
    with Harness() as h, h.client("input") as source, h.client() as admin:
        source.socket.sendall(b"x" * (1024 * 1024 + 1))
        try:
            closed = source.socket.recv(1)
            assert closed == b""
        except (ConnectionResetError, BrokenPipeError):
            pass
        assert admin.call("status")["pid"] == h.process.pid


def test_control_separate_from_data():
    with Harness() as h, h.client("output") as plugin, h.client("output", events=True) as slow, h.client() as admin:
        slow.call("subscribe", stream="raw")
        result = []
        def call():
            result.append(admin.call("control", target="output", method="inspect", args={"x": 7}))
        thread = threading.Thread(target=call)
        thread.start()
        control = plugin.receive()
        assert control["type"] == "control" and control["args"] == {"x": 7}
        plugin.call("reply", call_id=control["call_id"], result={"done": True}, error=None)
        thread.join(10)
        assert result == [{"done": True}]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--case")
    parser.add_argument("--report", type=Path, default=ROOT / "artifacts/protocol-results.json")
    args = parser.parse_args()
    tests = [(name, fn) for name, fn in globals().items() if name.startswith("test_") and callable(fn)]
    for name, fn in tests:
        if args.case and args.case not in name:
            continue
        started = time.monotonic()
        try:
            value = fn()
            result = {"case": name, "status": "skip" if isinstance(value, dict) and "skip" in value else "pass", "detail": value}
        except BaseException:
            result = {"case": name, "status": "fail", "error": traceback.format_exc()}
        result["seconds"] = round(time.monotonic() - started, 4)
        RESULTS.append(result)
        print(json.dumps(result, ensure_ascii=False), flush=True)
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps({"platform": os.sys.platform, "python": os.sys.version, "results": RESULTS}, indent=2))
    raise SystemExit(1 if any(r["status"] == "fail" for r in RESULTS) else 0)


if __name__ == "__main__":
    main()
