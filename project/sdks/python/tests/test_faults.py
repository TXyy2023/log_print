"""Protocol faults injected at a real TCP boundary; no mock of SDK internals."""
import asyncio
import contextlib
import json
import unittest

import log_print_sdk as log


class Peer:
    def __init__(self, behavior):
        self.behavior = behavior
        self.tasks = set()
        self.writers = set()
        self.requests = []
        self.seen = asyncio.Event()

    async def handle(self, reader, writer):
        task = asyncio.current_task()
        self.tasks.add(task)
        self.writers.add(writer)
        try:
            hello = json.loads(await reader.readline())
            self.send(writer, dict(type='response', id=0, error=None, result=dict(
                protocol='log-print/2', stream={'id': 's', 'alias': 'raw'}, reads=['other'])))
            while data := await reader.readline():
                message = json.loads(data)
                if message['op'] == 'report':
                    self.send(writer, dict(type='response', id=message['id'], result={'accepted': True}, error=None))
                    continue
                self.requests.append(message)
                self.seen.set()
                await self.behavior(self, reader, writer, message, hello)
        except (ConnectionError, asyncio.CancelledError):
            pass
        finally:
            writer.close()
            with contextlib.suppress(OSError):
                await writer.wait_closed()
            self.writers.discard(writer)
            self.tasks.discard(task)

    @staticmethod
    def send(writer, value):
        writer.write(json.dumps(value).encode() + b'\n')

    async def __aenter__(self):
        self.server = await asyncio.start_server(self.handle, '127.0.0.1', 0, limit=1024*1024)
        self.address = f'127.0.0.1:{self.server.sockets[0].getsockname()[1]}'
        return self

    async def __aexit__(self, *args):
        self.server.close()
        await self.server.wait_closed()
        for writer in tuple(self.writers):
            writer.close()
        tasks = tuple(self.tasks)
        for task in tasks:
            task.cancel()
        await asyncio.gather(*tasks, return_exceptions=True)

    def session(self, **options):
        return log.init(address=self.address, plugin='fixture', token='synthetic', **options)


class FaultTests(unittest.IsolatedAsyncioTestCase):
    async def test_timeout_unknown_does_not_retry_and_cancellation_keeps_full_frame(self):
        async def never_reply(peer, reader, writer, message, hello):
            pass
        async with Peer(never_reply) as peer:
            async with peer.session(timeout=.08) as app:
                with self.assertRaises(log.OutcomeUnknownError):
                    await app.send('once')
                self.assertEqual(len(peer.requests), 1)
                task = asyncio.create_task(app.send(b'x' * 65536))
                while len(peer.requests) < 2:
                    await asyncio.sleep(.005)
                task.cancel()
                with self.assertRaises(asyncio.CancelledError):
                    await task
                self.assertEqual(len(peer.requests[1]['args']['payload']), 65536)
                self.assertEqual(app._connection.pending, {})
                await asyncio.sleep(.1)
                self.assertEqual(len(peer.requests), 2)

    async def test_control_reply_not_starved_by_pending_request_slots(self):
        async def behavior(peer, reader, writer, message, hello):
            if message['op'] == 'publish' and len(peer.requests) == 32:
                peer.send(writer, dict(type='control', call_id=99, method='config.get', args={}))
            elif message['op'] == 'reply':
                peer.send(writer, dict(type='response', id=message['id'], result={}, error=None))
        async with Peer(behavior) as peer:
            async with peer.session(timeout=5) as app:
                sends = [asyncio.create_task(app.send(str(n))) for n in range(40)]
                try:
                    async with asyncio.timeout(2):
                        while not any(r['op'] == 'reply' for r in peer.requests):
                            await asyncio.sleep(.01)
                    reply = next(r for r in peer.requests if r['op'] == 'reply')
                    self.assertEqual(reply['args']['call_id'], 99)
                    self.assertTrue(reply['args']['result']['restart_required'])
                    self.assertLessEqual(len(app._connection.pending), 33)
                finally:
                    for task in sends:
                        task.cancel()
                    await asyncio.gather(*sends, return_exceptions=True)

    async def test_malformed_truncated_and_oversized_response_fail(self):
        for frame in (b'not-json\n', b'{"type":', b'x' * (1024*1024) + b'\n'):
            with self.subTest(frame_length=len(frame)):
                async def behavior(peer, reader, writer, message, hello):
                    writer.write(frame)
                    await writer.drain()
                    writer.close()
                async with Peer(behavior) as peer:
                    with self.assertRaises(log.SDKError):
                        async with peer.session(timeout=1) as app:
                            await app.send('request')

    async def test_gap_is_explicit_exception_with_range(self):
        async def behavior(peer, reader, writer, message, hello):
            if message['op'] == 'subscribe':
                peer.send(writer, dict(type='response', id=1, result={}, error=None))
                peer.send(writer, dict(type='gap', stream='other', epoch='e', **{'from': 4, 'to': 8}, reason='fixture'))
        async with Peer(behavior) as peer:
            async with peer.session() as app:
                with self.assertRaises(log.DataGapError) as caught:
                    await anext(app.records())
                self.assertEqual((caught.exception.start, caught.exception.end), (4, 8))

    async def test_large_integer_record_received_without_float_roundtrip(self):
        async def behavior(peer, reader, writer, message, hello):
            if message['op'] == 'subscribe':
                peer.send(writer, dict(type='response', id=1, result={}, error=None))
                peer.send(writer, dict(type='record', record=dict(stream='other', epoch='e', seq=2**64-2,
                    key='k', payload=[0,255], source_ts_ns=2**64-1, observed_ts_ns=2**64-1,
                    upstream={'parent': 2**63+1}, upstream_epochs={'parent': 'p'}, source_seq=2**64-1)))
        async with Peer(behavior) as peer:
            async with peer.session() as app:
                row = await anext(app.records())
                self.assertEqual(row.seq, 2**64-2)
                self.assertEqual(row.payload, b'\0\xff')
                self.assertEqual(row.upstream['parent'], 2**63+1)
