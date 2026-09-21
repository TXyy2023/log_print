#!/usr/bin/env python3
"""Real Core + Output acceptance. All bytes below are explicit test fixtures."""
import json
from contextlib import closing
import os
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import time
import unittest
from support import BIN, EXE, Core, eventually


def source():
    return {'id':'source','role':'input','bin':'unused','streams':[{'id':'raw'}]}


def output(binary, config, derived=False, attached=False):
    return {'id':'sink','role':'output','bin':binary,'reads':[] if attached else ['raw'],
            'streams':[{'id':'derived','parents':['raw']}] if derived else [],'config':config}


class Output:
    def __init__(self, core, plugin, env=None):
        self.core, self.plugin = core, plugin
        self.stdout_path = core.path / (plugin['id']+'.stdout')
        self.stderr_path = core.path / (plugin['id']+'.stderr')
        self.stdout = self.stdout_path.open('wb')
        self.stderr = self.stderr_path.open('wb')
        environ = dict(os.environ, LOG_PRINT_CORE=core.address, LOG_PRINT_PLUGIN=plugin['id'],
                       LOG_PRINT_TOKEN=plugin['id'], LOG_PRINT_CONFIG=json.dumps(plugin['config']),
                       LOG_PRINT_TRANSPORT=core.transport, **(env or {}))
        self.process = subprocess.Popen([str(BIN/(plugin['bin']+EXE))],env=environ,stdout=self.stdout,stderr=self.stderr)

    def report(self):
        with self.core.rpc() as admin:
            return next(p.get('report') or {} for p in admin.call('status')['plugins'] if p['id']==self.plugin['id'])

    def ready(self, state):
        def check():
            report=self.report()
            if report.get('state')==state: return report
            if self.process.poll() is not None: raise AssertionError((self.stderr_path.read_text(),report))
        return eventually(check)

    def stop(self):
        with self.core.rpc() as admin:
            reply=admin.call('control',target=self.plugin['id'],method='shutdown')
        assert self.process.wait(timeout=10)==0, self.stderr_path.read_text()
        return reply

    def __enter__(self): return self
    def __exit__(self,*args):
        if self.process.poll() is None:
            self.process.terminate()
            try: self.process.wait(timeout=8)
            except subprocess.TimeoutExpired:
                self.process.kill();self.process.wait(timeout=3)
                raise AssertionError('Output did not stop')
        self.stdout.close();self.stderr.close()


def read(core, owner):
    with core.rpc() as admin:
        return admin.call('read',stream=core.stream(owner),limit=64)['records']


def publish(core, writer, payload, seq=1, **extra):
    return writer.publish(core.stream('source'),payload,key=f'fixture:{seq}',source_seq=seq,**extra)


class Outputs(unittest.TestCase):
    def test_raw_dynamic_uuid_attachment_preserves_binary(self):
        plugin=output('output-raw',{'streams':['obsolete-config-value']},attached=True)
        with Core(plugins=[source(),plugin]) as core, core.rpc('source') as writer:
            publish(core,writer,b'\0\xfffirst\n',1)
            with core.rpc() as admin: admin.call('plugin.attach',plugin='sink',stream=core.stream('source'))
            with Output(core,plugin) as run:
                run.ready('displaying');publish(core,writer,b'second',2)
                eventually(lambda:run.stdout_path.read_bytes()==b'\0\xfffirst\nsecond')
                run.stop()
                self.assertEqual(run.stdout_path.read_bytes(),b'\0\xfffirst\nsecond')

    def archive_roundtrip(self, format):
        with tempfile.TemporaryDirectory(prefix='output-v2-') as directory:
            path=Path(directory)/'data';db=Path(directory)/'data.sqlite'
            config={'streams':['raw'],'mode':'create','file':{'format':format,'paths':{'raw':str(path)}},'sqlite':{'path':str(db)},'commit':{'max_records':2,'max_bytes':4096,'max_delay_ms':30}}
            plugin=output('output-file',config)
            with Core(plugins=[source(),plugin]) as core,core.rpc('source') as writer,Output(core,plugin) as run:
                run.ready('archiving')
                expected=[publish(core,writer,payload,index+1,channel='stdout',source_ts_ns=2**64-1) for index,payload in enumerate([b'\0\xff\n',b'',b'\xe4\xb8\xad'])]
                def confirmed():
                    with core.rpc() as admin:
                        status=admin.call('control',target='sink',method='status.get')
                        return status.get('common',{}).get(core.stream('source'),{}).get('next')==4
                eventually(confirmed)
                reply=run.stop();self.assertTrue(reply['stopped'])
                if format=='raw': self.assertEqual(path.read_bytes(),b'\0\xff\n\xe4\xb8\xad')
                else:
                    rows=[json.loads(line) for line in path.read_text().splitlines()]
                    self.assertTrue(all(r['format_version']==2 for r in rows));self.assertEqual([r['record'] for r in rows],expected)
                # sqlite3's context manager commits/rolls back but does not close
                # its file handle; Windows cannot remove the fixture until closed.
                with closing(sqlite3.connect(db)) as sql:
                    rows=sql.execute('SELECT payload,source_ts_ns,channel,source_seq FROM records ORDER BY length(seq),seq').fetchall()
                    self.assertEqual(rows,[(bytes(r['payload']),str(2**64-1),'stdout',str(i+1)) for i,r in enumerate(expected)])
                    self.assertEqual(sql.execute('PRAGMA integrity_check').fetchone()[0],'ok')
                self.assertEqual(run.stdout_path.read_bytes(),b'')

    def test_raw_and_sqlite_roundtrip_commit_and_shutdown(self): self.archive_roundtrip('raw')
    def test_jsonl_and_sqlite_roundtrip_complete_metadata(self): self.archive_roundtrip('jsonl')

    def test_archive_refuses_existing_targets_without_overwrite(self):
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'existing';path.write_bytes(b'keep-me')
            plugin=output('output-file',{'streams':['raw'],'mode':'create','file':{'format':'raw','paths':{'raw':str(path)}}})
            with Core(plugins=[source(),plugin]) as core,Output(core,plugin) as run:
                self.assertNotEqual(run.process.wait(timeout=8),0);self.assertEqual(path.read_bytes(),b'keep-me');self.assertEqual(run.report()['state'],'failed')

    def test_archive_sync_failure_does_not_report_saved(self):
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'data'
            plugin=output('output-file',{'streams':['raw'],'mode':'create','file':{'format':'raw','paths':{'raw':str(path)}},'commit':{'max_records':1,'max_bytes':4096,'max_delay_ms':20}})
            with Core(plugins=[source(),plugin]) as core,core.rpc('source') as writer,Output(core,plugin,{'LOG_PRINT_ARCHIVE_TESTING':'1','LOG_PRINT_ARCHIVE_ERRORPOINT':'file_sync'}) as run:
                run.ready('archiving');publish(core,writer,b'unconfirmed')
                self.assertNotEqual(run.process.wait(timeout=8),0);report=run.report();self.assertEqual(report['state'],'failed');self.assertFalse(report['complete']);self.assertIn('file_sync',report['error'])

    def test_transform_number_timestamp_reorders_and_leaves_original_unchanged(self):
        plugin=output('output-transform',{'streams':['raw'],'output_stream':'derived','number':True,'timestamp':True,'reorder':True,'max_delay_ms':1000},derived=True)
        with Core(plugins=[source(),plugin]) as core,core.rpc('source') as writer,Output(core,plugin) as run:
            run.ready('transforming')
            original=[publish(core,writer,text,seq,source_ts_ns=100+seq) for seq,text in [(3,b'three'),(1,b'one'),(2,b'two')]]
            rows=eventually(lambda:(r if len(r:=read(core,'sink'))==3 else None))
            self.assertEqual([bytes(r['payload']) for r in rows],[b'[n=1] [ts_ns=101] one',b'[n=2] [ts_ns=102] two',b'[n=3] [ts_ns=103] three'])
            self.assertEqual(read(core,'source'),original);self.assertNotEqual(core.stream('source'),core.stream('sink'));run.stop()

    def test_transform_missing_sequence_timeout_duplicate_and_shutdown_flush(self):
        plugin=output('output-transform',{'reorder':True,'max_delay_ms':120},derived=True)
        with Core(plugins=[source(),plugin]) as core,core.rpc('source') as writer,Output(core,plugin) as run:
            run.ready('transforming');publish(core,writer,b'three',3);publish(core,writer,b'duplicate',3)
            eventually(lambda:len(read(core,'sink'))==1)
            publish(core,writer,b'late',1);publish(core,writer,b'seven',7)
            # Shutdown accepts and flushes SDK events already delivered, including the pending gap.
            eventually(lambda:len(read(core,'source'))==4)
            eventually(lambda:len(read(core,'sink'))==2)
            run.stop();self.assertEqual([bytes(r['payload']) for r in read(core,'sink')],[b'three',b'seven'])
            report=run.report();self.assertEqual(report['state'],'stopped');self.assertEqual(report['duplicates'],2);self.assertEqual(report['pending'],0)

    def test_transform_shutdown_flushes_a_confirmed_pending_gap(self):
        plugin=output('output-transform',{'reorder':True,'max_delay_ms':60000},derived=True)
        with Core(plugins=[source(),plugin]) as core,core.rpc('source') as writer,Output(core,plugin) as run:
            run.ready('transforming')
            publish(core,writer,b'tenth-pending',10,channel='stdout')
            publish(core,writer,b'barrier',1,channel='stderr')
            # Receiving the following channel proves the previous event reached Processor.
            eventually(lambda:len(read(core,'sink'))==1)
            run.stop()
            self.assertEqual([bytes(r['payload']) for r in read(core,'sink')],[b'barrier',b'tenth-pending'])
            self.assertEqual(run.report()['skipped_source_sequences'],9)

    def test_archive_late_subscription_starts_at_retained_oldest(self):
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'retained'
            plugin=output('output-file',{'streams':['raw'],'mode':'create','file':{'format':'raw','paths':{'raw':str(path)}}})
            with Core(options={'buffer_records':2},plugins=[source(),plugin]) as core,core.rpc('source') as writer:
                for seq in range(1,6): publish(core,writer,str(seq).encode(),seq)
                with Output(core,plugin) as run:
                    run.ready('archiving');eventually(lambda:path.read_bytes()==b'45');run.stop()
                    self.assertEqual(path.read_bytes(),b'45')
                    self.assertEqual(run.report()['common'][core.stream('source')]['next'],6)

    def test_transform_stdout_stderr_source_sequences_are_independent(self):
        plugin=output('output-transform',{'reorder':True,'max_delay_ms':1000},derived=True)
        with Core(plugins=[source(),plugin]) as core,core.rpc('source') as writer,Output(core,plugin) as run:
            run.ready('transforming');publish(core,writer,b'out2',2,channel='stdout');publish(core,writer,b'err1',1,channel='stderr');publish(core,writer,b'out1',1,channel='stdout')
            rows=eventually(lambda:(r if len(r:=read(core,'sink'))==3 else None));run.stop()
            self.assertEqual([(r['channel'],r['source_seq'],bytes(r['payload'])) for r in rows],[('stderr',1,b'err1'),('stdout',1,b'out1'),('stdout',2,b'out2')])

    def test_transform_channel_limit_applies_without_reorder_or_source_sequence(self):
        for reorder, source_sequence in [(False, True), (True, False)]:
            with self.subTest(reorder=reorder,source_sequence=source_sequence):
                plugin=output('output-transform',{'reorder':reorder,'max_channels':1},derived=True)
                with Core(plugins=[source(),plugin]) as core,core.rpc('source') as writer,Output(core,plugin) as run:
                    run.ready('transforming')
                    extra={'source_seq':1} if source_sequence else {}
                    writer.publish(core.stream('source'),b'first',channel='first',**extra)
                    eventually(lambda:len(read(core,'sink'))==1)
                    writer.publish(core.stream('source'),b'over-limit',channel='second',**extra)
                    self.assertNotEqual(run.process.wait(timeout=8),0)
                    self.assertEqual(run.report()['state'],'failed')
                    self.assertIn('channel state limit',run.report()['error'])
                    self.assertEqual([bytes(r['payload']) for r in read(core,'sink')],[b'first'])

    def test_transform_expiry_uses_record_deadline_without_an_extra_timer_period(self):
        plugin=output('output-transform',{'reorder':True,'max_delay_ms':2000},derived=True)
        with Core(plugins=[source(),plugin]) as core,core.rpc('source') as writer,Output(core,plugin) as run:
            run.ready('transforming')
            # The former periodic timer fired at start; an arrival just afterwards
            # waited nearly two periods. Leave ample scheduling margin for CI.
            time.sleep(.15)
            started=time.monotonic()
            publish(core,writer,b'gap-at-two',2)
            eventually(lambda:len(read(core,'sink'))==1,timeout=5)
            self.assertLess(time.monotonic()-started,3.2)
            run.stop()

if __name__=='__main__': unittest.main(verbosity=2)
