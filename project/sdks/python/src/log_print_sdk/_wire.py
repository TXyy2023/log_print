"""Bounded TCP/JSONL implementation. Not a public Rust-shaped Client API."""
from __future__ import annotations

import asyncio
import contextlib
import json
from typing import Any

from ._models import (ConnectionLostError, OutcomeUnknownError, PayloadTooLargeError,
                      ProtocolError, SDKError, remote_error)

MAX_WIRE = 1024 * 1024
MAX_PAYLOAD = 64 * 1024


def encode(value: Any) -> bytes:
    data = json.dumps(value, separators=(',', ':'), ensure_ascii=True, allow_nan=False).encode() + b'\n'
    if len(data) > MAX_WIRE:
        raise PayloadTooLargeError('encoded frame exceeds 1 MiB', code='frame_too_large')
    return data


async def receive(reader: asyncio.StreamReader) -> dict[str, Any]:
    try:
        data = await reader.readuntil(b'\n')
    except asyncio.IncompleteReadError as error:
        if error.partial:
            raise ProtocolError('truncated TCP frame', code='truncated_frame') from error
        raise ConnectionLostError('Core connection closed', code='connection_lost') from error
    except asyncio.LimitOverrunError as error:
        raise ProtocolError('Core frame exceeds 1 MiB', code='frame_too_large') from error
    if len(data) > MAX_WIRE:
        raise ProtocolError('Core frame exceeds 1 MiB', code='frame_too_large')
    try:
        value = json.loads(data, parse_constant=lambda _: (_ for _ in ()).throw(ValueError('nonfinite JSON')))
        if not isinstance(value, dict):
            raise ValueError('object required')
        return value
    except (ValueError, UnicodeError) as error:
        raise ProtocolError('invalid Core JSON', code='invalid_json') from error


async def close_writer(writer: asyncio.StreamWriter) -> None:
    writer.close()
    with contextlib.suppress(OSError, TimeoutError):
        async with asyncio.timeout(2):
            await writer.wait_closed()


async def handshake(address: str, plugin: str, token: str, *, events: bool, timeout: float):
    writer = None
    try:
        host, port = address.rsplit(':', 1)
        async with asyncio.timeout(timeout):
            reader, writer = await asyncio.open_connection(host.strip('[]'), int(port), limit=MAX_WIRE - 1)
            writer.write(encode(dict(protocol='log-print/2', plugin=plugin, token=token, events=events)))
            await writer.drain()
            message = await receive(reader)
            if message.get('error'):
                raise remote_error(message['error'])
            result = message.get('result', {})
            if message.get('type') != 'response' or message.get('id') != 0 or result.get('protocol') != 'log-print/2':
                raise ProtocolError('invalid Core welcome', code='protocol_mismatch')
            return reader, writer, result
    except (OSError, ValueError) as error:
        if writer is not None:
            await close_writer(writer)
        if isinstance(error, ValueError):
            raise ProtocolError('invalid Core address or welcome', code='invalid_connection') from error
        raise ConnectionLostError('cannot connect/register with Core', code='connection_failed') from error
    except BaseException:
        if writer is not None:
            await close_writer(writer)
        raise


class Connection:
    def __init__(self, address, plugin, token, timeout):
        self.address, self.plugin, self.token, self.timeout = address, plugin, token, timeout
        self.pending = {}
        self.sequence = 0
        self.normal_slots = asyncio.Semaphore(32)
        self.control_slots = asyncio.Semaphore(8)
        self.write_lock = asyncio.Lock()
        self.controls = asyncio.Queue(32)
        self.failed = asyncio.Event()
        self.error: SDKError | None = None
        self.closed = False
        self.writer = None
        self.reader_task = None
        self.welcome = {}

    async def open(self):
        self.reader, self.writer, self.welcome = await handshake(
            self.address, self.plugin, self.token, events=False, timeout=min(10, self.timeout))
        self.reader_task = asyncio.create_task(self._read(), name='log-print-replies')
        return self

    def fail(self, error):
        if self.error is None:
            self.error = error
        self.failed.set()
        for future in tuple(self.pending.values()):
            if not future.done():
                future.set_exception(OutcomeUnknownError('Core connection lost; unconfirmed outcome unknown', code='connection_lost'))

    async def _read(self):
        try:
            while True:
                message = await receive(self.reader)
                kind = message.get('type')
                if kind == 'response':
                    future = self.pending.get(message.get('id'))
                    if future is not None and not future.done():
                        if message.get('error'):
                            future.set_exception(remote_error(message['error']))
                        else:
                            future.set_result(message.get('result'))
                elif kind == 'control':
                    try:
                        self.controls.put_nowait(message)
                    except asyncio.QueueFull as error:
                        raise ProtocolError('control queue exhausted', code='control_overflow') from error
                else:
                    raise ProtocolError('unexpected operation message', code='invalid_message')
        except asyncio.CancelledError:
            raise
        except Exception as error:
            self.fail(error if isinstance(error, SDKError) else ConnectionLostError('Core reader failed', code='connection_lost'))

    async def request(self, op, args):
        if self.closed or self.error:
            raise self.error or ConnectionLostError('session is closed', code='closed')
        slots = self.control_slots if op == 'reply' else self.normal_slots
        future = None
        sent = False
        identity = None
        try:
            async with asyncio.timeout(self.timeout):
                async with slots:
                    self.sequence += 1
                    identity = self.sequence
                    data = encode(dict(id=identity, op=op, args=args))
                    future = asyncio.get_running_loop().create_future()
                    self.pending[identity] = future
                    async with self.write_lock:
                        if self.closed or self.error:
                            raise self.error or ConnectionLostError('session is closed', code='closed')
                        # One synchronous write enqueues the entire frame. Cancellation at
                        # drain cannot truncate it or cause a duplicate request.
                        self.writer.write(data)
                        sent = True
                        await self.writer.drain()
                    return await future
        except TimeoutError as error:
            if sent:
                raise OutcomeUnknownError('request timed out; outcome unknown, no automatic retry', code='timeout_unknown') from error
            raise SDKError('request timed out before transmission', code='timeout_not_sent') from error
        except OSError as error:
            self.fail(ConnectionLostError('Core transport failed', code='connection_lost'))
            raise OutcomeUnknownError('transport failed; outcome unknown', code='connection_lost') from error
        finally:
            if identity is not None:
                self.pending.pop(identity, None)
            if future is not None:
                if not future.done():
                    future.cancel()
                elif not future.cancelled():
                    future.exception()  # Retrieve failures even if cancellation won the race.

    async def close(self):
        self.closed = True
        self.fail(ConnectionLostError('session closed', code='closed'))
        if self.reader_task:
            self.reader_task.cancel()
            await asyncio.gather(self.reader_task, return_exceptions=True)
        if self.writer:
            await close_writer(self.writer)
