"""Python-facing async sessions and record iterators."""
from __future__ import annotations

import asyncio
from collections.abc import Callable, Mapping
import contextlib
import copy
import json
import math
import os
from typing import Any
import uuid

from ._local import LocalCore
from ._models import (ConfigurationError, ConnectionLostError, DataGapError, ProtocolError,
                      PayloadTooLargeError, Record, SDKError, StoppedError, remote_error, u64)
from ._wire import Connection, MAX_PAYLOAD, close_writer, encode, handshake, receive


class Records:
    """Bounded async iterator. Use aclose() to release a subscription early."""
    def __init__(self, session: Session, stream: str | None):
        self.session = session
        self.stream = stream
        self.queue: asyncio.Queue[Record] = asyncio.Queue(session.queue_size)
        self.tasks: list[asyncio.Task] = []
        self.writers = []
        self.claimed: list[str] = []
        self.changed = asyncio.Event()
        self.error: BaseException | None = None
        self.started = False
        self.closed = False
        self.reading = False

    def __aiter__(self):
        return self

    async def _start(self):
        self.started = True
        session = self.session
        session._check()
        connection = session._output or session._connection
        streams = [await session._resolve(self.stream)] if self.stream is not None else connection.welcome.get('reads', [])
        if not streams:
            raise ConfigurationError('No default readable streams; declare reads in the plugin configuration', code='no_read_streams')
        for stream in streams:
            if stream in session._subscriptions:
                raise ConfigurationError(f'already subscribed to {stream}', code='already_subscribed')
        # No await between duplicate check and claiming all identities.
        for stream in streams:
            session._subscriptions.add(stream)
            self.claimed.append(stream)
        for stream in streams:
            reader, writer, _ = await handshake(connection.address, connection.plugin, connection.token,
                                                 events=True, timeout=min(10, session.timeout))
            try:
                async with asyncio.timeout(session.timeout):
                    writer.write(encode(dict(id=1, op='subscribe', args=dict(stream=stream))))
                    await writer.drain()
                    message = await receive(reader)
                    if message.get('error'):
                        raise remote_error(message['error'])
                    if message.get('type') != 'response' or message.get('id') != 1:
                        raise ProtocolError('invalid subscription response', code='invalid_message')
                self.writers.append(writer)
                self.tasks.append(asyncio.create_task(self._pump(reader, writer), name='log-print-records'))
            except BaseException:
                await close_writer(writer)
                raise

    async def _pump(self, reader, writer):
        try:
            while True:
                message = await receive(reader)
                if message.get('type') == 'record':
                    await self.queue.put(Record._parse(message['record']))
                elif message.get('type') == 'gap':
                    raise DataGapError(message)
                elif message.get('error'):
                    raise remote_error(message['error'])
                else:
                    raise ProtocolError('unexpected subscription message', code='invalid_message')
        except asyncio.CancelledError:
            raise
        except Exception as error:
            self.error = error
            self.changed.set()
        finally:
            await close_writer(writer)

    async def __anext__(self) -> Record:
        if self.reading:
            raise RuntimeError('concurrent reads on one iterator are not supported')
        if self.closed:
            raise StopAsyncIteration
        self.reading = True
        waits = []
        try:
            if not self.started:
                await self._start()
            waits = [asyncio.create_task(self.queue.get()),
                     asyncio.create_task(self.changed.wait()),
                     asyncio.create_task(self.session._stop.wait())]
            await asyncio.wait(waits, return_when=asyncio.FIRST_COMPLETED)
            if self.error:
                raise self.error
            if self.session._error:
                raise self.session._error
            if self.closed or self.session.stopping:
                raise StopAsyncIteration
            return waits[0].result()
        except BaseException:
            await self.aclose()
            raise
        finally:
            for task in waits:
                task.cancel()
            await asyncio.gather(*waits, return_exceptions=True)
            self.reading = False

    async def aclose(self) -> None:
        if self.closed:
            return
        self.closed = True
        self.changed.set()
        for task in self.tasks:
            task.cancel()
        await asyncio.gather(*self.tasks, return_exceptions=True)
        for writer in self.writers:
            await close_writer(writer)
        for stream in self.claimed:
            self.session._subscriptions.discard(stream)
        self.session._iterators.discard(self)


class Session:
    """One managed plugin, or an isolated owned Core with input/output identities.

    Construct with init(); enter with async with before calling send/records.
    A Session can be entered once and must be used on its owning event loop.
    """
    def __init__(self, *, address: str | None = None, plugin: str | None = None,
                 token: str | None = None, config: Mapping[str, Any] | None = None,
                 validate: Callable[[dict[str, Any]], Mapping[str, Any]] | None = None,
                 core_binary: str | os.PathLike | None = None,
                 core_options: Mapping[str, Any] | None = None,
                 timeout: float = 30, queue_size: int = 64):
        if isinstance(timeout, bool) or not isinstance(timeout, (int, float)) or not math.isfinite(timeout) or timeout <= 0:
            raise ValueError('timeout must be a positive finite number')
        if type(queue_size) is not int or not 1 <= queue_size <= 4096:
            raise ValueError('queue_size must be 1..4096')
        self.timeout, self.queue_size = timeout, queue_size
        self._explicit = (address, plugin, token)
        self._initial_config, self._validate = config, validate
        self._core_binary, self._core_options = core_binary, core_options
        self._connection: Connection | None = None
        self._output: Connection | None = None
        self._local: LocalCore | None = None
        self._config: dict[str, Any] = {}
        self._controllers: list[asyncio.Task] = []
        self._iterators: set[Records] = set()
        self._subscriptions: set[str] = set()
        self._stop = asyncio.Event()
        self._error: SDKError | None = None
        self._entered = False
        self._closed = False
        self._key_prefix = uuid.uuid4().hex
        self._key_sequence = 0

    async def __aenter__(self) -> Session:
        if self._entered:
            raise RuntimeError('Session cannot be entered more than once')
        self._entered = True
        try:
            explicit = any(v is not None for v in self._explicit)
            managed = any(k in os.environ for k in ('LOG_PRINT_CORE', 'LOG_PRINT_PLUGIN', 'LOG_PRINT_TOKEN'))
            if explicit and not all(isinstance(v, str) and v for v in self._explicit):
                raise ConfigurationError('address, plugin and token must be provided together', code='incomplete_identity')
            if not explicit and managed and not all(os.environ.get(k) for k in ('LOG_PRINT_CORE', 'LOG_PRINT_PLUGIN', 'LOG_PRINT_TOKEN')):
                raise ConfigurationError('incomplete LOG_PRINT environment', code='incomplete_identity')
            if not explicit and managed and os.environ.get('LOG_PRINT_TRANSPORT', 'tcp') != 'tcp':
                raise ConfigurationError('Python SDK requires a TCP instance', code='unsupported_transport')
            if (explicit or managed) and (self._core_binary is not None or self._core_options is not None):
                raise ConfigurationError('core_binary/core_options apply only to an owned local instance', code='invalid_configuration')
            config = self._initial_config
            if config is None:
                config = json.loads(os.environ.get('LOG_PRINT_CONFIG', '{}')) if managed and not explicit else {}
            if config is None:  # Omitted PluginSpec.config is JSON null in the Rust host.
                config = {}
            if not isinstance(config, Mapping):
                raise ConfigurationError('config must be a mapping', code='invalid_configuration')
            self._config = copy.deepcopy(dict(config))
            if self._validate:
                validated = self._validate(copy.deepcopy(self._config))
                if not isinstance(validated, Mapping):
                    raise ConfigurationError('validate must return a mapping', code='invalid_configuration')
                self._config = copy.deepcopy(dict(validated))
            encode(self._config)  # Reject non-JSON / nonfinite config before creating resources.
            if explicit:
                address, plugin, token = self._explicit
            elif managed:
                address, plugin, token = (os.environ[k] for k in ('LOG_PRINT_CORE', 'LOG_PRINT_PLUGIN', 'LOG_PRINT_TOKEN'))
            else:
                self._local = LocalCore(self._core_binary, self._core_options)
                await self._local.start()
                address, plugin, token = self._local.address, 'python-input', self._local.tokens['python-input']
            self._connection = Connection(address, plugin, token, self.timeout)
            await self._connection.open()
            if self._local:
                self._output = Connection(address, 'python-output', self._local.tokens['python-output'], self.timeout)
                await self._output.open()
            for connection in (self._connection, self._output):
                if connection:
                    self._controllers.append(asyncio.create_task(self._control(connection), name='log-print-control'))
                    await connection.request('report', dict(state='sdk_ready', business_complete=False))
            return self
        except BaseException:
            await self.close()
            raise

    async def __aexit__(self, kind, error, traceback):
        if not self.stopping and self._connection and self._connection.error:
            self._error = self._connection.error
        try:
            if error is not None and isinstance(error, Exception) and self._connection:
                # Report type, not possibly secret-bearing exception values or configs.
                with contextlib.suppress(Exception):
                    async with asyncio.timeout(2):
                        await self.report(state='failed', error=type(error).__name__)
            elif error is None and self._connection and not self._error:
                with contextlib.suppress(Exception):
                    async with asyncio.timeout(2):
                        await self.report(state='stopped', business_complete=False)
        finally:
            await self.close()
        if error is None and self._error:
            raise self._error

    @property
    def config(self) -> dict[str, Any]:
        """A copy of the validated startup snapshot; mutation cannot change it."""
        return copy.deepcopy(self._config)

    @property
    def stream(self) -> str | None:
        owned = self._connection.welcome.get('stream') if self._connection else None
        return owned.get('id') if owned else None

    @property
    def stopping(self) -> bool:
        return self._stop.is_set()

    def _check(self):
        if not self._connection or self._closed:
            raise ConfigurationError('use the session inside async with init()', code='session_not_open')
        if self._error:
            raise self._error
        if self._connection.error:
            raise self._connection.error
        if self.stopping:
            raise StoppedError('plugin is stopping', code='stopping')

    async def _control(self, connection):
        failure = asyncio.create_task(connection.failed.wait())
        pending = None
        try:
            while not self.stopping:
                pending = asyncio.create_task(connection.controls.get())
                await asyncio.wait([pending, failure], return_when=asyncio.FIRST_COMPLETED)
                if failure.done():
                    raise connection.error or ConnectionLostError('control connection closed')
                control = pending.result()
                method = control.get('method')
                error = None
                if method == 'shutdown':
                    result = dict(stopping=True, completed=False)
                elif method == 'config.get':
                    result = dict(effective=self.config, dynamic_fields=[], restart_required=True)
                else:
                    result = None
                    error = dict(code='restart_required' if method == 'config.patch' else 'invalid_control',
                                 message='configuration requires restart' if method == 'config.patch' else 'unsupported control')
                await connection.request('reply', dict(call_id=control['call_id'], result=result, error=error))
                if method == 'shutdown':
                    self._stop.set()
        except asyncio.CancelledError:
            raise
        except Exception as error:
            self._error = error if isinstance(error, SDKError) else ConnectionLostError('control processing failed', code='connection_lost')
            self._stop.set()
        finally:
            failure.cancel()
            if pending:
                pending.cancel()
            await asyncio.gather(failure, *([pending] if pending else []), return_exceptions=True)

    async def sleep(self, seconds: float) -> None:
        """Sleep until the interval elapses or shutdown arrives. Faults still raise."""
        self._check()
        try:
            async with asyncio.timeout(seconds):
                await self._stop.wait()
        except TimeoutError:
            pass
        if self._error:
            raise self._error

    async def streams(self) -> list[dict[str, Any]]:
        self._check()
        offset, rows = 0, {}
        while True:
            page = await self._connection.request('streams', dict(offset=offset, limit=64))
            if isinstance(page, list):
                return page
            for row in page['streams']:
                rows[row['id']] = row
            next_offset = page['next']
            if next_offset >= page['total']:
                return list(rows.values())
            if next_offset <= offset:
                raise ProtocolError('catalog cursor did not advance', code='invalid_cursor')
            offset = next_offset

    async def _resolve(self, stream: str) -> str:
        if not isinstance(stream, str) or not stream:
            raise ValueError('stream must be a nonempty alias or UUID')
        for row in await self.streams():
            if stream in (row['id'], row.get('alias')):
                return row['id']
        raise ConfigurationError(f'unknown stream: {stream}', code='unknown_stream')

    async def send(self, data: str | bytes, *, stream: str | None = None,
                   key: str | None = None, channel: str | None = None,
                   source_seq: int | None = None, source_ts_ns: int | None = None,
                   upstream: Mapping[str, int] | None = None) -> Record:
        self._check()
        if not isinstance(data, (str, bytes)):
            raise TypeError('send accepts str or bytes; serialize other values explicitly')
        payload = data.encode('utf-8') if isinstance(data, str) else data
        if len(payload) > MAX_PAYLOAD:
            raise PayloadTooLargeError('payload exceeds 64 KiB', code='payload_too_large')
        for name, value in (('source_seq', source_seq), ('source_ts_ns', source_ts_ns)):
            if value is not None:
                u64(value, name)
        if channel is not None and not isinstance(channel, str):
            raise TypeError('channel must be str or None')
        positions = dict(upstream or {})
        for name, value in positions.items():
            if not isinstance(name, str):
                raise TypeError('upstream keys must be stream UUIDs')
            u64(value, 'upstream position')
        target = await self._resolve(stream) if stream is not None else self.stream
        if target is None:
            raise ConfigurationError('No default writable stream; declare a derived stream for an Output', code='no_write_stream')
        if key is None:
            self._key_sequence += 1
            key = f'{self._key_prefix}:{self._key_sequence}'
        if not isinstance(key, str):
            raise TypeError('key must be str')
        result = await self._connection.request('publish', dict(stream=target, key=key, payload=list(payload),
            channel=channel, source_seq=source_seq, source_ts_ns=source_ts_ns, upstream=positions))
        return Record._parse(result)

    def records(self, *, stream: str | None = None) -> Records:
        self._check()
        iterator = Records(self, stream)
        self._iterators.add(iterator)
        return iterator

    async def read_range(self, stream: str, *, epoch: str, start: int, end: int, limit: int = 64) -> dict[str, Any]:
        """Read retained memory only. Keeps Core's explicit coverage metadata."""
        self._check()
        u64(start, 'start')
        u64(end, 'end')
        if type(limit) is not int or not 1 <= limit <= 64:
            raise ValueError('limit must be 1..64')
        target = await self._resolve(stream)
        connection = self._output or self._connection
        result = await connection.request('read.range', dict(stream=target, epoch=epoch, **{'from': start}, end=end, limit=limit))
        return dict(result, records=[Record._parse(r) for r in result['records']])

    async def create_stream(self, description: str, *, parents: list[str]) -> dict[str, Any]:
        self._check()
        resolved = [await self._resolve(parent) for parent in parents]
        created = await self._connection.request('stream.create', dict(description=description, parents=resolved))
        self._connection.welcome['stream'] = created
        return created

    async def report(self, **fields: Any) -> None:
        """Report business state; the library never invents successful completion."""
        if not self._connection or self._closed:
            raise ConfigurationError('session is not open', code='session_not_open')
        await self._connection.request('report', fields)

    async def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        self._stop.set()
        for task in self._controllers:
            task.cancel()
        await asyncio.gather(*self._controllers, return_exceptions=True)
        for iterator in tuple(self._iterators):
            await iterator.aclose()
        for connection in (self._output, self._connection):
            if connection:
                await connection.close()
        if self._local:
            await self._local.close()


def init(*, address: str | None = None, plugin: str | None = None,
         token: str | None = None, config: Mapping[str, Any] | None = None,
         validate: Callable[[dict[str, Any]], Mapping[str, Any]] | None = None,
         core_binary: str | os.PathLike | None = None,
         core_options: Mapping[str, Any] | None = None,
         timeout: float = 30, queue_size: int = 64) -> Session:
    """Create a Session (no I/O until async context entry).

    Defaults: managed LOG_PRINT_* environment when present, otherwise an isolated
    local Core discovered on PATH/LOG_PRINT_CORE_BIN. See Session for keywords.
    """
    return Session(address=address, plugin=plugin, token=token, config=config,
                   validate=validate, core_binary=core_binary, core_options=core_options,
                   timeout=timeout, queue_size=queue_size)
