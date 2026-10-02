#!/usr/bin/env python3
"""Shared contract + actual PTY/ConPTY terminal acceptance on all three OSes."""
import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path
import webui
from webui import request
from cli import NativeApp
from support import BIN, EXE, ROOT, eventually, RPC
webui.KIND='tui'
WebUI=webui.WebUI

def control(app,identity,method,**args):
    state=json.loads(app.state.read_text())
    with RPC(state['address'],'__manager__',state['token']) as manager:
        return manager.call('core.call',op='control',args={'target':identity,'method':method,'args':args})

class Terminal:
    def __init__(self,url):
        self.process=subprocess.Popen([str(BIN/'examples'/('pty_driver'+EXE)),str(BIN/('output-tui'+EXE)),url],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,encoding='utf-8',cwd=ROOT)
        assert json.loads(self.process.stdout.readline())['ready']
    def call(self,**args):
        self.process.stdin.write(json.dumps(args)+'\n');self.process.stdin.flush()
        line=self.process.stdout.readline()
        if not line:raise AssertionError(self.process.stderr.read())
        return json.loads(line)
    def __enter__(self):return self
    def __exit__(self,*args):
        self.process.stdin.close()
        try:self.process.wait(timeout=12)
        except subprocess.TimeoutExpired:self.process.kill();self.process.wait(timeout=5)
        error=self.process.stderr.read();self.process.stdout.close();self.process.stderr.close()
        if args[0] is None:assert self.process.returncode==0,error

class TerminalAcceptance(unittest.TestCase):
    def test_attach_keys_mouse_resize_conflict_detach_and_webui_sync(self):
        # Attaching to WebUI must manipulate the exact same Page, not copy it.
        with NativeApp('--output-webui','web') as app:
            url=eventually(lambda:control(app,'web','url').get('url'))
            app.cli('tui','web','page','set','--sidebar-open','false','--view-x','0','--view-y','0','--theme','dark')
            app.cli('tui','web','panel','add','--title','PTY console','--left','0','--top','0','--panel-width','640','--panel-height','320')
            state=request(url);panel=state['pages'][0]['panels'][0]['id']
            app.cli('tui','web','page','set','--active-panel',panel)
            with Terminal(url) as term:
                self.assertTrue(term.call(wait='PTY console')['alternate'])
                term.call(send='?',wait='TERMINAL WORKBENCH');term.call(send='\x1b')
                term.call(send=':panel set --title "终端设置"\r',wait='Saved · panel.set')
                eventually(lambda:request(url)['pages'][0]['panels'][0]['title']=='终端设置')
                # Captured edit revision must not overwrite concurrent CLI changes.
                term.call(send='/stale-filter',wait='Text filter')
                app.cli('webui','web','panel','set','--panel',panel,'--text','from-cli')
                term.call(send='\r',wait='revision_conflict')
                self.assertEqual(request(url)['pages'][0]['panels'][0]['text'],'from-cli')
                term.call(send=':panel set --text ""\r',wait='Saved · panel.set')
                term.call(send='m\x1b[C\r')
                eventually(lambda:request(url)['pages'][0]['panels'][0]['left']==8)
                # SGR mouse title drag, from (10,4) to (14,6).
                term.call(wait=f"rev {request(url)['revision']}")
                term.call(send='\x1b[<0;10;4M\x1b[<32;14;6M\x1b[<0;14;6m')
                eventually(lambda:request(url)['pages'][0]['panels'][0]['left']==40)
                self.assertIn('Terminal too small',term.call(resize=[30,10],wait='Terminal too small')['screen'])
                term.call(resize=[140,36],wait='终端设置')
                # A second view observes CLI state without creating a new backend.
                with Terminal(url) as mirror:
                    mirror.call(wait='终端设置')
                    app.cli('tui','web','page','set','--title','SYNCHRONIZED')
                    term.call(wait='SYNCHRONIZED');mirror.call(wait='SYNCHRONIZED')
                    result=mirror.call(send='\x03',finish=True)
                    self.assertEqual(result['exit'],0);self.assertFalse(result['alternate'])
                result=term.call(send='q',finish=True)
                self.assertEqual(result['exit'],0);self.assertFalse(result['alternate'])
                if os.name!='nt':self.assertTrue(result['restored'])
            self.assertEqual(request(url)['pages'][0]['title'],'SYNCHRONIZED')
            # Real viewer exits without putting pipes into terminal modes.
            result=app.cli('tui','web','attach',ok=False)
            self.assertNotEqual(result.returncode,0);self.assertIn('interactive terminal',result.stderr)
            snapshot=app.cli('tui','web','attach','--snapshot','--width','140','--height','36')
            self.assertIn('SYNCHRONIZED',snapshot.stdout);self.assertNotIn('\x1b',snapshot.stdout)

    def test_tui_archive_snapshot_history_and_connection_failure(self):
        with tempfile.TemporaryDirectory() as td:
            source=Path(td)/'source.log';source.write_text('temperature=20\nERROR 早期记录\n',encoding='utf-8')
            with NativeApp('--input-file',f'source={source}','--set','source.from_start=true','--output-tui','term','--tui-archive',f'term={td}/capture') as app:
                url=eventually(lambda:control(app,'term','url').get('url'))
                app.cli('tui','term','page','set','--sidebar-open','false','--view-x','0','--view-y','0')
                app.cli('tui','term','panel','add','--title','Archived logs','--left','0','--top','0','--panel-width','900','--panel-height','400','--column','text')
                with Terminal(url) as term:
                    term.call(wait='ERROR 早期记录')
                    term.call(send='h',wait='HISTORY')
                    term.call(send='o',wait='archive_and_memory');term.call(send='\x1b')
                    term.call(send='l',wait='LIVE')
                    term.call(send=' ',wait='PAUSED')
                    with source.open('a',encoding='utf-8') as file:file.write('continued-after-pause\n')
                    self.assertNotIn('continued-after-pause',term.call(wait='PAUSED')['screen'])
                    term.call(send=' ',wait='continued-after-pause')
                    app.cli('plugin','stop','term')
                    term.call(wait='DISCONNECTED',timeout_ms=15000)
                    self.assertEqual(term.call(send='q',finish=True)['exit'],0)
                app.cli('plugin','start','term')
                app.cli('tui','term','attach','--snapshot')

if __name__=='__main__':unittest.main(verbosity=2)
