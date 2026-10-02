"""Installed-wheel acceptance against real Rust processes and fault-injecting peers."""
import asyncio
import contextlib
import importlib.resources
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import log_print_sdk as log

ROOT = Path(os.environ['LOG_PRINT_SDK_TEST_ROOT'])
sys.path.insert(0, str(ROOT / 'quality/tests/v2'))
from support import Core, RPC, App, BIN, EXE, eventually


def connect(core, name, **options):
    return log.init(address=core.address, plugin=name, token=name, **options)


def call(core, op, **args):
    with core.rpc() as admin:
        return admin.call(op, **args)


class PublicTests(unittest.IsolatedAsyncioTestCase):
    async def test_installed_wheel_defaults_exact_bytes_and_owned_cleanup(self):
        self.assertNotIn(str(ROOT / 'project/sdks/python/src'), log.__file__)
        self.assertTrue(importlib.resources.files(log).joinpath('py.typed').is_file())
        async with log.init() as app:
            process = app._local.process
            temp = app._local.temp.name
            self.assertIsNone(process.returncode)
            first = await app.send('中文\n', source_seq=2**64-1, source_ts_ns=2**64-1, channel='stdout')
            second = await app.send(b'\0\xff\n')
            records = app.records()
            actual = await asyncio.wait_for(anext(records), 3)
            self.assertEqual(actual, first)
            self.assertEqual(actual.source_seq, 2**64-1)
            self.assertEqual(actual.text(), '中文\n')
            actual = await asyncio.wait_for(anext(records), 3)
            self.assertEqual(actual, second)
            with self.assertRaises(UnicodeDecodeError):
                actual.text()
            await records.aclose()
            again = app.records()
            self.assertEqual(await anext(again), first)
        self.assertIsNotNone(process.returncode)
        self.assertFalse(Path(temp).exists())
        self.assertFalse([t for t in asyncio.all_tasks() if t is not asyncio.current_task() and t.get_name().startswith('log-print-')])

    async def test_context_exception_and_cancel_cleanup(self):
        with self.assertRaisesRegex(ValueError, 'business'):
            async with log.init() as app:
                process = app._local.process
                raise ValueError('business')
        self.assertIsNotNone(process.returncode)
        ready = asyncio.Event()
        holder = []
        async def work():
            async with log.init() as app:
                holder.append(app._local.process)
                ready.set()
                await anext(app.records())
        task = asyncio.create_task(work())
        await ready.wait()
        task.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await task
        self.assertIsNotNone(holder[0].returncode)

    async def test_validation_no_core_and_static_config(self):
        with self.assertRaises(log.ConfigurationError):
            async with log.init(core_binary='missing-log-print-core-binary'):
                pass
        with patch.dict(os.environ, {'LOG_PRINT_CORE': '127.0.0.1:1'}):
            with self.assertRaises(log.ConfigurationError):
                async with log.init():
                    pass
        with self.assertRaisesRegex(ValueError, 'invalid business'):
            async with log.init(validate=lambda _: (_ for _ in ()).throw(ValueError('invalid business'))):
                pass
        async with log.init(config={'nested': {'n': 1}}) as app:
            snapshot = app.config
            snapshot['nested']['n'] = 99
            self.assertEqual(app.config['nested']['n'], 1)
            for data in (None, {}, 123):
                with self.assertRaises(TypeError):
                    await app.send(data)
            with self.assertRaises(ValueError):
                await app.send(b'x', source_seq=2**64)
            with self.assertRaises(log.PayloadTooLargeError):
                await app.send(b'x' * 65537)
            with self.assertRaises(log.PayloadTooLargeError):
                await app.send('x', key='x' * (1024 * 1024))
            self.assertEqual((await app.send('still usable')).seq, 1)

    async def test_duplicate_subscription_and_cancelled_wait_release(self):
        async with log.init() as app:
            await app.send('one')
            first = app.records()
            await anext(first)
            with self.assertRaises(log.ConfigurationError):
                await anext(app.records())
            blocked = asyncio.create_task(anext(first))
            await asyncio.sleep(.03)
            blocked.cancel()
            with self.assertRaises(asyncio.CancelledError):
                await blocked
            replacement = app.records()
            self.assertEqual((await anext(replacement)).text(), 'one')
            await replacement.aclose()
            await replacement.aclose()

    async def test_empty_stream_tail_waits_then_delivers(self):
        async with log.init() as app:
            pending = asyncio.create_task(anext(app.records()))
            await asyncio.sleep(.1)
            self.assertFalse(pending.done())
            await app.send('later')
            self.assertEqual((await asyncio.wait_for(pending, 3)).text(), 'later')

    async def test_buffer_coverage_and_epoch_error(self):
        async with log.init(core_options={'buffer_records': 2}) as app:
            for n in range(5):
                record = await app.send(str(n))
            page = await app.read_range(app.stream, epoch=record.epoch, start=1, end=6)
            self.assertEqual([r.seq for r in page['records']], [4, 5])
            self.assertEqual(page['uncovered_before'], {'first': 1, 'last': 3})
            with self.assertRaises(log.SDKError):
                await app.read_range(app.stream, epoch='wrong', start=1, end=6)

    async def test_owned_core_path_with_spaces_and_bad_options_cleanup(self):
        import shutil
        with tempfile.TemporaryDirectory(prefix='sdk binary space ') as directory:
            target = Path(directory) / ('owned core' + EXE)
            shutil.copy2(os.environ['LOG_PRINT_CORE_BIN'], target)
            async with log.init(core_binary=target) as app:
                self.assertEqual((await app.send('path')).text(), 'path')
        session = log.init(core_options={'buffer_records': 0})
        with self.assertRaises(log.ConfigurationError):
            async with session:
                pass
        self.assertIsNotNone(session._local.process.returncode)
        self.assertIsNone(session._local.temp)

    async def test_managed_binary_and_derived_stream_and_permissions(self):
        plugins = [dict(id='a', role='input', bin='unused', streams=[dict(id='raw')]),
                   dict(id='out', role='output', bin='unused', reads=['raw'],
                        streams=[dict(id='derived', parents=['raw'])])]
        with Core(plugins=plugins) as core:
            async with connect(core, 'a') as source, connect(core, 'out') as sink:
                first = await source.send(b'\0\xffsource', source_seq=2**53+7)
                received = await anext(sink.records())
                self.assertEqual(first, received)
                result = await sink.send('derived', upstream={received.stream: received.seq})
                self.assertEqual(result.upstream, {received.stream: received.seq})
                self.assertEqual(result.upstream_epochs, {received.stream: received.epoch})
                with self.assertRaises(log.PermissionDeniedError):
                    await source.send('bad', stream='derived')
                self.assertIsNone(source._local)
            self.assertIsNone(core.process.poll())

    async def test_config_control_and_shutdown_while_event_queue_full(self):
        with Core() as core:
            async with connect(core, 'a') as source, connect(core, 'out', config={'a': 1}, queue_size=1) as sink:
                await source.send('first')
                records = sink.records(stream='raw')
                await anext(records)
                for _ in range(12):
                    await source.send('queued')
                await asyncio.sleep(.05)
                config = await asyncio.to_thread(call, core, 'control', target='out', method='config.get')
                self.assertEqual(config['effective'], {'a': 1})
                with self.assertRaises(Exception) as caught:
                    await asyncio.to_thread(call, core, 'control', target='out', method='config.patch', args={'a': 2})
                self.assertEqual(caught.exception.code, 'restart_required')
                reply = await asyncio.to_thread(call, core, 'control', target='out', method='shutdown')
                self.assertEqual(reply, {'stopping': True, 'completed': False})
                with self.assertRaises(StopAsyncIteration):
                    await anext(records)

    async def test_disconnect_is_failure_not_eof(self):
        with Core() as core:
            with self.assertRaises(log.ConnectionLostError):
                async with connect(core, 'out') as app:
                    pending = asyncio.create_task(anext(app.records()))
                    await asyncio.sleep(.05)
                    await asyncio.to_thread(core.stop)
                    await pending

    async def test_managed_environment_and_wrong_identity(self):
        with Core() as core:
            env = dict(LOG_PRINT_CORE=core.address, LOG_PRINT_PLUGIN='a', LOG_PRINT_TOKEN='a',
                       LOG_PRINT_CONFIG='{"x":3}', LOG_PRINT_TRANSPORT='tcp')
            with patch.dict(os.environ, env):
                async with log.init() as app:
                    self.assertEqual(app.config, {'x': 3})
                    self.assertIsNone(app._local)
                    self.assertEqual((await app.send('environment')).seq, 1)
            with patch.dict(os.environ, dict(env, LOG_PRINT_TRANSPORT='udp')):
                with self.assertRaises(log.ConfigurationError) as caught:
                    async with log.init():
                        pass
                self.assertEqual(caught.exception.code, 'unsupported_transport')
            with self.assertRaises(log.SDKError):
                async with log.init(address=core.address, plugin='out', token='wrong'):
                    pass

    async def test_dynamic_derived_default_and_business_failure_report(self):
        with Core() as core:
            async with connect(core, 'a') as source, connect(core, 'out') as sink:
                origin = await source.send('source')
                created = await sink.create_stream('dynamic', parents=['raw'])
                result = await sink.send('derived', upstream={origin.stream: origin.seq})
                self.assertEqual(result.stream, created['id'])
                self.assertEqual(sink.stream, created['id'])
            with self.assertRaisesRegex(RuntimeError, 'business'):
                async with connect(core, 'other-out'):
                    raise RuntimeError('business secret that must not enter report')
            report = await asyncio.to_thread(call, core, 'plugin.status', plugin='other-out')
            self.assertEqual(report['report'], {'state': 'failed', 'error': 'RuntimeError'})

    async def test_directory_pagination(self):
        plugins = [dict(id=f'p{n}', role='input', bin='unused', streams=[dict(id=f's{n}')]) for n in range(70)]
        with Core(plugins=plugins) as core:
            async with connect(core, 'p0') as app:
                self.assertEqual(len(await app.streams()), 70)
                self.assertEqual((await app.send('alias', stream='s0')).stream, app.stream)


class ProcessTests(unittest.TestCase):
    def test_python_input_to_rust_raw_output(self):
        plugins = [dict(id='a', role='input', bin='unused', streams=[dict(id='raw')]),
                   dict(id='out', role='output', bin='output-raw', reads=['raw'], config={'streams': ['raw']})]
        with Core(plugins=plugins) as core:
            with (core.path / 'raw.bin').open('wb') as output, (core.path / 'stderr.txt').open('wb') as errors:
                env = dict(os.environ, LOG_PRINT_CORE=core.address, LOG_PRINT_PLUGIN='out', LOG_PRINT_TOKEN='out',
                           LOG_PRINT_TRANSPORT='tcp', LOG_PRINT_CONFIG=json.dumps(plugins[1]['config']))
                proc = subprocess.Popen([str(BIN / ('output-raw'+EXE))], env=env, stdout=output, stderr=errors)
                try:
                    eventually(lambda: (call(core, 'plugin.status', plugin='out').get('report') or {}).get('state') == 'displaying')
                    async def publish():
                        async with connect(core, 'a') as app:
                            await app.send(b'\0\xffhello\n')
                            await app.send('中文')
                    asyncio.run(publish())
                    expected = b'\0\xffhello\n' + '中文'.encode()
                    eventually(lambda: (core.path / 'raw.bin').read_bytes() == expected)
                    call(core, 'control', target='out', method='shutdown')
                    self.assertEqual(proc.wait(timeout=8), 0)
                finally:
                    if proc.poll() is None:
                        proc.kill()
                    proc.wait(timeout=8)

    def test_rust_input_to_python_output(self):
        with Core() as core:
            file = core.path / 'fixture bytes.bin'
            file.write_bytes(b'\0\xffhello\n' + '中文'.encode())
            env = dict(os.environ, LOG_PRINT_CORE=core.address, LOG_PRINT_PLUGIN='a', LOG_PRINT_TOKEN='a',
                       LOG_PRINT_TRANSPORT='tcp', LOG_PRINT_CONFIG=json.dumps({'path': str(file), 'mode': 'static'}))
            with (core.path / 'input.stderr').open('wb') as errors:
                proc = subprocess.Popen([str(BIN / ('input-file'+EXE))], env=env, stdout=subprocess.DEVNULL, stderr=errors)
                try:
                    self.assertEqual(proc.wait(timeout=8), 0)
                    async def consume():
                        async with connect(core, 'out') as app:
                            result = bytearray()
                            async with asyncio.timeout(5):
                                async for record in app.records(stream='raw'):
                                    result.extend(record.payload)
                                    if len(result) == len(file.read_bytes()):
                                        break
                            self.assertEqual(bytes(result), file.read_bytes())
                    asyncio.run(consume())
                finally:
                    if proc.poll() is None:
                        proc.kill()
                    proc.wait(timeout=8)

    def test_supervisor_launches_examples_and_stops(self):
        examples = ROOT / 'project/sdks/python/examples'
        with tempfile.TemporaryDirectory(prefix='python plugins ') as tmp:
            path = Path(tmp) / 'received.jsonl'
            config = dict(core={'transport': 'tcp'}, plugins=[
                dict(id='source', role='input', bin=sys.executable,
                     args=[str(examples / 'input.py')], streams=[dict(id='raw')]),
                dict(id='transform', role='output', bin=sys.executable,
                     args=[str(examples / 'transform.py')], reads=['raw'],
                     streams=[dict(id='upper', parents=['raw'])]),
                dict(id='sink', role='output', bin=sys.executable,
                     args=[str(examples / 'output.py')], reads=['upper'], config={'path': str(path)}),
            ])
            with App(config) as app:
                eventually(lambda: path.exists() and 'TEMPERATURE' in path.read_text())
                rows = [json.loads(line) for line in path.read_text().splitlines()]
                self.assertTrue(rows[0]['upstream'])
                status = app.inspect('status')
                self.assertTrue(status)
