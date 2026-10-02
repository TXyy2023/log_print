#!/usr/bin/env python3
"""Real-process WebUI/SQLite/CLI acceptance; no browser mocks or CDN."""
import json
from contextlib import closing
import re
import sqlite3
import sys
import tempfile
import time
import unittest
import urllib.request
import urllib.error
from pathlib import Path
from support import Core, RPC, eventually
from outputs import Output
from cli import NativeApp


def request(url, method=None, args=None, origin=True):
    data=json.dumps({'method':method,'args':args or {}}).encode() if method else None
    headers={'Content-Type':'application/json'}
    if origin: headers['Origin']=url
    try:
        with urllib.request.urlopen(urllib.request.Request(url+('/api/control' if method else '/api/state'),data,headers),timeout=10) as r:
            return json.load(r)
    except urllib.error.HTTPError as error:
        error.add_note(error.read().decode()); error.close(); raise


def job(url, method='history.read', **args):
    return request(url,method,args)['query']


def complete(url, identity, timeout=15):
    def check():
        value=request(url,'query.get',{'query':identity})
        state=value['status']['state']
        if state=='failed': raise AssertionError(value)
        return value if state!='running' else None
    return eventually(check,timeout)


def all_rows(url, identity, timeout=15):
    value=complete(url,identity,timeout);rows=[];offset=0
    while offset<value['total']:
        page=request(url,'query.get',{'query':identity,'offset':offset})
        assert page['next']>offset,page
        rows.extend(page['rows']);offset=page['next']
    return rows,value


def base(web_config, archive=None):
    plugins=[{'id':'source','role':'input','bin':'unused','streams':[{'id':'raw'}]},
             {'id':'derive','role':'output','bin':'unused','reads':['raw']},
             {'id':'web','role':'output','bin':'output-webui','read_all':True,'config':web_config}]
    if archive:plugins.append(archive)
    return plugins


class WebUI(unittest.TestCase):
    def test_memory_only_http_sse_revision_and_empty_streams(self):
        with tempfile.TemporaryDirectory() as td:
            config={'state_path':str(Path(td)/'pages.sqlite3')}
            plugins=base(config)
            with Core(options={'buffer_records':2},plugins=plugins) as core,Output(core,plugins[2]) as web:
                url=web.ready('serving')['url']
                eventually(lambda:len(request(url)['streams'])==1)
                with urllib.request.urlopen(url) as response:
                    html=response.read().decode();self.assertIn('/assets/',html);self.assertNotIn('cdn',html)
                with urllib.request.urlopen(url+'/api/events',timeout=5) as sse:
                    self.assertIn(b'event: state',sse.readline())
                    self.assertIn(b'pages',sse.readline())
                with self.assertRaises(urllib.error.HTTPError) as e:request(url,'page.create',{'name':'blocked'},False)
                self.assertEqual(e.exception.code,403)
                revision=request(url)['revision']
                request(url,'page.create',{'name':'saved','revision':revision})
                with self.assertRaises(urllib.error.HTTPError) as e:request(url,'page.set',{'title':'lost','revision':revision})
                self.assertEqual(e.exception.code,409)
                with core.rpc('source') as writer:
                    writer.call('stream.claim',stream=core.stream('source'))
                    for i in range(1,15):writer.publish(core.stream('source'),f'value={i}\n'.encode());time.sleep(.005)
                identity=job(url)
                rows,value=all_rows(url,identity)
                self.assertEqual(value['status']['coverage']['mode'],'memory_only')
                self.assertIsNone(value['status']['coverage']['archive_id'])
                self.assertTrue(rows)
                self.assertFalse(value['status']['coverage']['streams'][0]['history_available'])
                web.stop()

    def test_archive_overflow_dynamic_streams_search_context_curve_fixed_boundary(self):
        with tempfile.TemporaryDirectory() as td:
            path=Path(td)/'history.sqlite'
            arch={'id':'archive','role':'output','bin':'output-file','read_all':True,
                  'config':{'mode':'create','streams':[],'discover_streams':True,'sqlite':{'path':str(path)},'fail_on_gap':False,'commit':{'max_records':1,'max_delay_ms':10}}}
            config={'state_path':str(Path(td)/'pages.sqlite3'),'history_path':str(path),'history_plugin':'archive'}
            plugins=base(config,arch)
            with Core(options={'buffer_records':4},plugins=plugins) as core,Output(core,arch) as archive,Output(core,plugins[2]) as web:
                archive.ready('archiving');url=web.ready('serving')['url']
                with core.rpc('source') as writer:
                    stream=core.stream('source');writer.call('stream.claim',stream=stream)
                    for i in range(1,421):writer.publish(stream,f'value={i}\n'.encode(),channel='stdout');time.sleep(.003)
                    with closing(sqlite3.connect(path)) as db:eventually(lambda:db.execute('SELECT next FROM checkpoints WHERE stream=?',(stream,)).fetchone()[0]=='421')
                    with core.rpc() as admin:
                        info=admin.call('stream.get',stream=stream);self.assertGreater(info['oldest'],100)
                    query=job(url,streams=[stream]);snapshot=complete(url,query)
                    for i in range(421,431):writer.publish(stream,f'value={i}\n'.encode(),channel='stdout');time.sleep(.003)
                    rows,fixed=all_rows(url,query)
                    self.assertEqual([r['text'] for r in rows],[f'value={i}' for i in range(1,421)])
                    identities=[(r['stream'],r['epoch'],r['seq'],r['offset']) for r in rows];self.assertEqual(len(identities),len(set(identities)))
                    early,_=all_rows(url,job(url,'history.search',streams=[stream],regex='^value=1$'))
                    self.assertEqual(early[0]['seq'],'1')
                    ns=int(early[0]['observed_ts_ns'])
                    exact,_=all_rows(url,job(url,'history.search',streams=[stream],regex='^value=1$',time_from=str(ns),time_end=str(ns)))
                    self.assertEqual(len(exact),1)
                    outside,_=all_rows(url,job(url,'history.search',streams=[stream],regex='^value=1$',time_from=str(ns+1)))
                    self.assertEqual(outside,[])
                    context,_=all_rows(url,job(url,'history.context',streams=[stream],seq=2,before=1,after=1))
                    self.assertEqual([r['text'] for r in context],['value=1','value=2','value=3'])
                    curve,_=all_rows(url,job(url,'history.curve',streams=[stream],regex=r'value=(?P<value>\d+)'))
                    self.assertEqual(curve[0]['value'],1);self.assertEqual(curve[-1]['value'],430);self.assertLessEqual(len(curve),2000)
                    with core.rpc('derive') as derived:
                        new=derived.call('stream.create',description='dynamic',parents=[stream])['id']
                        with closing(sqlite3.connect(path)) as db:eventually(lambda:db.execute('SELECT 1 FROM streams WHERE stream=?',(new,)).fetchone())
                        derived.publish(new,b'value=999\n',upstream={stream:430})
                        with closing(sqlite3.connect(path)) as db:eventually(lambda:db.execute('SELECT COUNT(*) FROM records WHERE stream=?',(new,)).fetchone()[0]==1)
                        self.assertTrue(any(s['id']==new for s in eventually(lambda:request(url) if len(request(url)['streams'])==2 else None)['streams']))
                        panel=request(url,'panel.add',{'kind':'log','streams':[new]})['result']['id']
                        def bound_rows():
                            data=request(url,'panel.data',{'panel':panel})
                            return data['rows'] if data['rows'] else None
                        self.assertEqual([r['text'] for r in eventually(bound_rows)],['value=999'])
                    writer.publish(stream,b'val',channel='stdout');writer.publish(stream,b'err\n',channel='stderr');writer.publish(stream,b'ue=777\n',channel='stdout');time.sleep(.15)
                    split,_=all_rows(url,job(url,'history.search',streams=[stream],text='777'))
                    self.assertEqual(split[0]['text'],'value=777');self.assertNotEqual(split[0]['seq'],split[0]['end_seq'])
                    archive.stop()
                    eventually(lambda:request(url)['archive_writer']['connected'] is False)
                    stopped=complete(url,job(url,streams=[stream]))
                    self.assertFalse(stopped['status']['coverage']['writer']['connected'])
                    self.assertTrue(stopped['status']['coverage']['streams'][0]['history_available'])
                    with self.assertRaises(urllib.error.HTTPError):job(url,streams=[stream],epoch='old-runtime')
                    with closing(sqlite3.connect(path)) as db:
                        db.execute("UPDATE metadata SET value=? WHERE key='archive_id'",(__import__('uuid').uuid4().__str__(),));db.commit()
                    with self.assertRaises(urllib.error.HTTPError):job(url,streams=[stream])
                web.stop()

    def test_late_archive_write_failure_and_coverage(self):
        with tempfile.TemporaryDirectory() as td:
            path=Path(td)/'late.sqlite'
            archive={'id':'archive','role':'output','bin':'output-file','read_all':True,'config':{'mode':'create','discover_streams':True,'sqlite':{'path':str(path)},'fail_on_gap':False,'commit':{'max_records':1}}}
            plugins=base({'state_path':str(Path(td)/'pages.sqlite3'),'history_path':str(path),'history_plugin':'archive'},archive)
            with Core(options={'buffer_records':2},plugins=plugins) as core, Output(core,plugins[2]) as web:
                url=web.ready('serving')['url']
                with core.rpc('source') as writer:
                    stream=core.stream('source');writer.call('stream.claim',stream=stream)
                    for i in range(1,7):writer.publish(stream,f'early={i}\n'.encode());time.sleep(.01)
                    before=complete(url,job(url,streams=[stream]))
                    self.assertTrue(before['status']['coverage']['archive_error'])
                    with Output(core,archive,{'LOG_PRINT_ARCHIVE_TESTING':'1','LOG_PRINT_ARCHIVE_ERRORPOINT':'sqlite_commit','LOG_PRINT_ARCHIVE_TEST_DELAY_MS':'100'}) as file:
                        file.ready('archiving')
                        writer.publish(stream,b'after archive\n')
                        eventually(lambda:file.process.poll() is not None)
                        self.assertNotEqual(file.process.returncode,0)
                        # Health polling can observe registration before the
                        # first report; wait for the failure report to arrive.
                        eventually(lambda:((request(url).get('archive_writer') or {}).get('report') or {}).get('state')=='failed')
                        failed=complete(url,job(url,streams=[stream]))
                        coverage=failed['status']['coverage']
                        self.assertEqual(coverage['writer']['report']['state'],'failed')
                        self.assertIn('commit',coverage['writer']['report']['error'])
                        self.assertEqual(coverage['streams'][0]['archived']['first'],'5')
                        self.assertEqual(coverage['streams'][0]['archived']['last'],'4')
                        self.assertIsNotNone(coverage['streams'][0]['uncommitted'])
                web.stop()

    def test_udp_budget_and_byte_context(self):
        with tempfile.TemporaryDirectory() as td:
            plugin=base({'state_path':str(Path(td)/'pages.sqlite3')})
            with Core(transport='udp',options={'buffer_records':64},plugins=plugin) as core,Output(core,plugin[2]) as web:
                url=web.ready('serving')['url']
                with core.rpc('source') as writer:
                    stream=core.stream('source');writer.call('stream.claim',stream=stream)
                    # Large, explicit fixtures: one record contains many logical lines.
                    writer.publish(stream,b'zero\none\ntwo\nthree\nfour\n')
                    time.sleep(.05)
                    with core.rpc() as admin:
                        epoch=admin.call('stream.get',stream=stream)['epoch']
                        result=admin.call('read.range',stream=stream,epoch=epoch,**{'from':1,'end':1,'limit':64})
                        self.assertEqual(len(result['records']),1)
                    context,_=all_rows(url,job(url,'history.context',streams=[stream],seq=1,byte_offset=9,before=1,after=1))
                    self.assertEqual([r['text'] for r in context],['one','two','three'])
                    # UDP control pages fit the transport and leave the service usable.
                    for i in range(10):writer.publish(stream,(b'x'*300+b'\n')*20);time.sleep(.02)
                    eventually(lambda:request(url)['streams'][0]['head']==11)
                    identity=job(url,streams=[stream])
                    complete(url,identity)
                    with core.rpc() as admin:
                        reply=admin.call('control',target='web',method='query.get',args={'query':identity})
                        self.assertLess(len(json.dumps(reply).encode()),60*1024)
                        self.assertLess(reply['next'],reply['total'])
                        self.assertTrue(admin.call('control',target='web',method='url')['url'].startswith('http://'))
                web.stop()

    def test_background_cancel_and_concurrency(self):
        with tempfile.TemporaryDirectory() as td:
            plugins=base({'state_path':str(Path(td)/'pages.sqlite3')})
            with Core(options={'buffer_records':64},plugins=plugins) as core,Output(core,plugins[2]) as web:
                url=web.ready('serving')['url']
                with core.rpc('source') as writer:
                    stream=core.stream('source');writer.call('stream.claim',stream=stream)
                    for i in range(20):writer.publish(stream,b'x\n'*3000)
                    # Plenty of logical rows, but a bounded number of retained records.
                    q1=job(url,streams=[stream]);q2=job(url,streams=[stream])
                    with self.assertRaises(urllib.error.HTTPError) as busy:job(url,streams=[stream])
                    self.assertEqual(busy.exception.code,429)
                    request(url,'query.cancel',{'query':q1});request(url,'query.cancel',{'query':q2})
                    self.assertEqual(complete(url,q1)['status']['state'],'cancelled')
                    self.assertEqual(complete(url,q2)['status']['state'],'cancelled')
                web.stop()

    def test_pause_is_a_shared_backend_frame_while_collection_continues(self):
        with tempfile.TemporaryDirectory() as td:
            plugins=base({'state_path':str(Path(td)/'pages.sqlite3')})
            with Core(options={'buffer_records':2},plugins=plugins) as core,Output(core,plugins[2]) as web:
                url=web.ready('serving')['url']
                with core.rpc('source') as writer:
                    stream=core.stream('source');writer.call('stream.claim',stream=stream)
                    for i in range(1,10):writer.publish(stream,f'value={i}\n'.encode());time.sleep(.01)
                    panel=request(url,'panel.add',{'kind':'log'})['result']['id']
                    eventually(lambda:request(url,'panel.data',{'panel':panel})['rows'])
                    request(url,'panel.set',{'panel':panel,'paused':True})
                    frozen=request(url,'panel.data',{'panel':panel})
                    self.assertTrue(frozen['frozen'])
                    for i in range(10,31):writer.publish(stream,f'value={i}\n'.encode());time.sleep(.005)
                    self.assertEqual(request(url,'panel.data',{'panel':panel})['rows'],frozen['rows'])
                    with core.rpc() as other_viewer:
                        self.assertEqual(other_viewer.call('control',target='web',method='panel.data',args={'panel':panel})['rows'],frozen['rows'])
                        self.assertEqual(other_viewer.call('stream.get',stream=stream)['head'],30)
                    request(url,'panel.set',{'panel':panel,'paused':False})
                    eventually(lambda:request(url,'panel.data',{'panel':panel})['rows'][-1]['text']=='value=30')
                web.stop()

    def test_udp_catalog_is_complete_when_descriptions_exceed_one_frame(self):
        with tempfile.TemporaryDirectory() as td:
            plugins=base({'state_path':str(Path(td)/'pages.sqlite3')})
            for i in range(24):
                plugins.append({'id':f'extra-{i}','role':'input','bin':'unused','streams':[{'id':f'flow-{i}','description':'x'*4000}]})
            with Core(transport='udp',plugins=plugins) as core,Output(core,plugins[2]) as web:
                url=web.ready('serving')['url']
                eventually(lambda:len(request(url)['streams'])==25)
                with core.rpc() as admin:
                    first=admin.call('control',target='web',method='streams',args={'offset':0})
                    self.assertLess(first['next'],first['total'])
                    following=admin.call('control',target='web',method='streams',args={'offset':first['next']})
                    self.assertEqual(following['next'],25)
                web.stop()

    def test_slow_archive_keeps_gaps_and_later_context(self):
        with tempfile.TemporaryDirectory() as td:
            path=Path(td)/'gaps.sqlite'
            archive={'id':'archive','role':'output','bin':'output-file','read_all':True,
                     'config':{'mode':'create','discover_streams':True,'sqlite':{'path':str(path)},'fail_on_gap':False,
                               'queue':{'max_records':1},'commit':{'max_records':1}}}
            plugins=base({'state_path':str(Path(td)/'pages.sqlite3'),'history_path':str(path),'history_plugin':'archive'},archive)
            # Bound Core's event queue too, and make the writer slower than the
            # publisher on all CI hosts. Small records at 5 ms can be absorbed
            # entirely by TCP/SDK queues, producing no actual eviction gap.
            with Core(options={'buffer_records':1,'queue_records':1},plugins=plugins) as core,Output(core,archive,{'LOG_PRINT_ARCHIVE_TESTING':'1','LOG_PRINT_ARCHIVE_TEST_DELAY_MS':'50'}) as file:
                file.ready('archiving')
                with core.rpc('source') as writer:
                    stream=core.stream('source');writer.call('stream.claim',stream=stream)
                    for i in range(1,501):writer.publish(stream,b'p'*(48*1024)+f'\nvalue={i}\n'.encode(),channel='stdout')
                with closing(sqlite3.connect(path)) as db:
                    eventually(lambda:db.execute('SELECT next FROM checkpoints WHERE stream=?',(stream,)).fetchone()[0]=='501',15)
                    self.assertGreater(db.execute('SELECT COUNT(*) FROM gaps').fetchone()[0],0)
                # Start after the overwrite so the WebUI cache cannot repair archive gaps.
                with Output(core,plugins[2]) as web:
                    url=web.ready('serving')['url']
                    # The pressure fixture scans up to 24 MiB in debug builds.
                    # Keep ordinary queries at 15s; allow bounded extra time
                    # here for Windows runners to format the large log rows.
                    rows,value=all_rows(url,job(url,streams=[stream]),timeout=60)
                    self.assertGreater(value['status']['coverage']['gap_count'],0)
                    self.assertEqual(rows[-1]['text'],'value=500')
                    self.assertTrue(any(r.get('kind')=='gap' for r in rows))
                    curve,_=all_rows(url,job(url,'history.curve',streams=[stream],channels=['stdout'],text='value=',time_from='1',regex=r'value=(?P<value>\d+)'),timeout=60)
                    self.assertTrue(any(p.get('gap') and p['value'] is None for p in curve))
                    self.assertEqual(curve[-1]['value'],500)
                    web.stop()
                file.stop()

    def test_cli_pages_restore_launch_modes_and_archive_flags(self):
        with tempfile.TemporaryDirectory() as td:
            source=Path(td)/'app.log';source.write_text('')
            options=('--input-file',f'source={source}','--output-webui','web','--webui-archive',f'web={td}/capture')
            with NativeApp(*options) as app:
                def control(method,**args):
                    state=json.loads(app.state.read_text())
                    with RPC(state['address'],'__manager__',state['token']) as manager:
                        return manager.call('core.call',op='control',args={'target':'web','method':method,'args':args})
                url=control('url')['url']
                app.cli('webui','web','page','create','--name','dashboard','--title','监控')
                app.cli('webui','web','page','select','--page','dashboard')
                app.cli('webui','web','panel','add','--page','dashboard','--title','Logs','--kind','log','--h','7','--stream','raw-waiting')
                state=request(url);panel=state['pages'][-1]['panels'][0]['id']
                app.cli('webui','web','panel','set','--panel',panel,'--text','saved-filter','--paused','true','--format','hex','--x','1','--w','11')
                app.cli('webui','web','panel','set','--panel',panel,'--column','time','--column','seq','--column','text','--column-width','text=650','--sort-column','seq','--sort-order','desc')
                saved=control('panel.get',panel=panel)['panel']
                self.assertEqual(next(c['width'] for c in saved['column_state'] if c['colId']=='text'),650)
                self.assertEqual(next(c['sort'] for c in saved['column_state'] if c['colId']=='seq'),'desc')
                app.cli('webui','web','page','clone','--page','dashboard','--name','copy')
                app.cli('webui','web','page','delete','--page','copy')
                before=control('page.get',page='dashboard')['page']
                app.cli('plugin','restart','web');url=control('url')['url']
                self.assertEqual(control('page.get',page='dashboard')['page'],before)
                app.cli('stop');app.started=False
                app.start();app.started=True;url=control('url')['url']
                after=control('page.get',page='dashboard')['page'];self.assertEqual(before,after)
                self.assertEqual(request(url)['selected'],after['id'])
                archives=list((Path(td)/'capture').glob('run-*.sqlite'));self.assertEqual(len(archives),2)
                with closing(sqlite3.connect(archives[0])) as db:
                    self.assertEqual(db.execute("SELECT value FROM metadata WHERE key='schema_version'").fetchone()[0],'3')
            with NativeApp('--input-file',f'source={source}','--output-sqlite',f'archive={td}/explicit.sqlite','--output-webui','web','--webui-history','web=archive') as app:
                status=app.inspect('status');self.assertIn('archive',str(status));self.assertTrue((Path(td)/'explicit.sqlite').exists())
            invalid=NativeApp('--output-webui','web','--webui-archive',f'web={td}/a','--webui-history','web=archive')
            self.assertNotEqual(invalid.cli('start',*invalid.options,ok=False).returncode,0)
            invalid.temp.cleanup()

if __name__=='__main__':unittest.main(verbosity=2)
