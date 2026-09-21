#!/usr/bin/env python3
"""Actual Core + official Input process acceptance, including isolated tmux.

Fixtures deliberately generate bytes; these are test records, not software logs.
"""
import contextlib
import json
import os
from pathlib import Path
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import time

from support import BIN, EXE, ROOT, Core, eventually

RESULTS = []


def spec(binary, config, name='source'):
    return dict(id=name, role='input', bin=binary,
                streams=[dict(id=name+'-alias', description='input acceptance fixture')], config=config)


class Input:
    def __init__(self, core, plugin, extra_env=None):
        self.core, self.plugin = core, plugin
        self.log_path = core.path / (plugin['id']+'.stderr')
        self.log = open(self.log_path, 'wb')
        env = dict(os.environ, LOG_PRINT_CORE=core.address, LOG_PRINT_PLUGIN=plugin['id'],
                   LOG_PRINT_TOKEN=plugin['id'], LOG_PRINT_TRANSPORT=core.transport,
                   LOG_PRINT_CONFIG=json.dumps(plugin['config']))
        env.update(extra_env or {})
        self.process = subprocess.Popen([str(BIN / (plugin['bin']+EXE))], env=env,
                                        stdout=subprocess.DEVNULL, stderr=self.log)

    def report(self):
        with self.core.rpc() as admin:
            return next(p.get('report') or {} for p in admin.call('status')['plugins']
                        if p['id']==self.plugin['id'])

    def ready(self, state):
        def check():
            value = self.report()
            if value.get('state') == state:
                return value
            if self.process.poll() is not None:
                raise AssertionError(f'Input exited before {state}: {self.log_path.read_text()} {value}')
            return None
        return eventually(check)

    def wait(self, success=True):
        code = self.process.wait(timeout=10)
        assert (code==0) == success, (code, self.log_path.read_text())
        return self.report()

    def stop(self):
        with self.core.rpc() as admin:
            admin.call('control', target=self.plugin['id'], method='shutdown')
        self.wait()

    def close(self):
        if self.process.poll() is None:
            self.process.terminate()
        try:
            self.process.wait(timeout=8)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait(timeout=3)
            raise AssertionError('Input did not stop on termination')
        self.log.close()

    def __enter__(self): return self
    def __exit__(self, *args): self.close()


def records(core, owner='source'):
    with core.rpc() as admin:
        return admin.call('read', stream=core.stream(owner), limit=64)['records']


def payload(core, channel=None):
    return b''.join(bytes(r['payload']) for r in records(core)
                    if channel is None or r.get('channel') == channel)


def static_binary(directory):
    source = directory/'bytes.bin'
    data = bytes(range(256))*101 + b'without-newline'
    source.write_bytes(data)
    plugin = spec('input-file', dict(path=str(source), mode='static', chunk_bytes=701))
    with Core(plugins=[plugin]) as core, Input(core, plugin) as run:
        report = run.wait()
        assert report['state']=='source_eof' and report['bytes_sent']==len(data)
        assert report['downstream_complete'] is False
        assert payload(core)==data
        rows = records(core)
        assert [r['source_seq'] for r in rows]==list(range(1,len(rows)+1))
        assert {r['stream'] for r in rows}=={core.stream('source')}
        assert all(r['channel'] is None for r in rows)
        assert len(core.streams())==1
        # Source exit neither removes the stream nor waits for an Output.
        time.sleep(.05)
        assert payload(core)==data


def static_empty(directory):
    source = directory/'empty'
    source.touch()
    plugin = spec('input-file', dict(path=str(source),mode='static'))
    with Core(plugins=[plugin]) as core, Input(core, plugin) as run:
        report = run.wait()
        assert report['bytes_sent']==0 and report['chunks_sent']==0
        assert core.streams()[0]['head']==0 and records(core)==[]


def follow(directory):
    source = directory/'tail.log'
    source.write_bytes(b'OLD history must be skipped')
    plugin = spec('input-file', dict(path=str(source),poll_ms=10))
    with Core(plugins=[plugin]) as core, Input(core, plugin) as run:
        run.ready('following')
        assert payload(core)==b''
        expected = b'append'
        with source.open('ab') as writer: writer.write(expected)
        eventually(lambda: payload(core)==expected)
        # Plain truncation followed by append starts a new segment.
        source.write_bytes(b'x')
        expected += b'x'
        eventually(lambda: payload(core)==expected)
        # Same-size rewrite is detected by the last-read anchor.
        source.write_bytes(b'y')
        expected += b'y'
        eventually(lambda: payload(core)==expected)
        source.rename(directory/'tail.log.1')
        eventually(lambda: run.report().get('state')=='path_missing')
        source.write_bytes(b'new-inode')
        expected += b'new-inode'
        eventually(lambda: payload(core)==expected)
        run.stop()
        rows = records(core)
        assert len(core.streams())==1
        assert [r['source_seq'] for r in rows]==list(range(1,len(rows)+1))
        assert len({r['key'].split(':')[1] for r in rows})==4


def from_start(directory):
    source = directory/'existing.log'
    source.write_bytes(b'initial bytes')
    plugin = spec('input-file', dict(path=str(source),from_start=True,poll_ms=10))
    with Core(plugins=[plugin]) as core, Input(core, plugin) as run:
        eventually(lambda: payload(core)==b'initial bytes')
        run.stop()


def program_channels(directory):
    source = directory/'source.py'
    source.write_text("import os,sys\nassert sys.stdin.read()==''\nassert 'LOG_PRINT_TOKEN' not in os.environ\nos.write(1,b'A\\x00\\xffwithout-newline')\nos.write(2,b'err\\x00\\xfe')\n")
    plugin = spec('input-program', dict(command=sys.executable,args=['-u',str(source)],chunk_bytes=3))
    with Core(plugins=[plugin]) as core, Input(core, plugin) as run:
        report = run.wait()
        assert report['state']=='source_exited' and report['exit_code']==0
        assert payload(core,'stdout')==b'A\x00\xffwithout-newline'
        assert payload(core,'stderr')==b'err\x00\xfe'
        rows = records(core)
        assert len(core.streams())==1
        assert [r['seq'] for r in rows]==list(range(1,len(rows)+1))
        for channel in ['stdout','stderr']:
            channel_rows=[r for r in rows if r['channel']==channel]
            assert [r['source_seq'] for r in channel_rows]==list(range(1,len(channel_rows)+1))
            assert all(':'+channel+':' in r['key'] for r in channel_rows)


def nonzero(directory):
    plugin = spec('input-program', dict(command=sys.executable,args=['-c',"import os,sys;os.write(2,b'failure detail');sys.exit(7)"]))
    with Core(plugins=[plugin]) as core, Input(core, plugin) as run:
        report=run.wait(success=False)
        assert report['state']=='failed' and '7' in report['error']
        assert payload(core,'stderr')==b'failure detail'


def process_alive(pid):
    if os.name=='nt':
        result=subprocess.run(['tasklist','/FI',f'PID eq {pid}','/FO','CSV','/NH'],capture_output=True,text=True,timeout=5)
        return f'"{pid}"' in result.stdout
    try: os.kill(pid,0)
    except ProcessLookupError: return False
    # A killed descendant awaiting its parent's/system reaper is not running.
    result=subprocess.run(['ps','-o','stat=','-p',str(pid)],capture_output=True,text=True,timeout=3)
    return bool(result.stdout.strip()) and not result.stdout.strip().startswith('Z')


def stop_tree(directory):
    marker=directory/'pids.json'
    source=directory/'parent.py'
    source.write_text("import json,os,pathlib,subprocess,sys,time\nchild=subprocess.Popen([sys.executable,'-c','import time;time.sleep(60)'])\npathlib.Path(sys.argv[1]).write_text(json.dumps([os.getpid(),child.pid]))\nos.write(1,b'parent ready')\ntime.sleep(60)\n")
    plugin=spec('input-program',dict(command=sys.executable,args=['-u',str(source),str(marker)]))
    with Core(plugins=[plugin]) as core, Input(core,plugin) as run:
        run.ready('capturing')
        pids=eventually(lambda: json.loads(marker.read_text()) if marker.exists() and marker.stat().st_size else None)
        assert all(process_alive(pid) for pid in pids)
        eventually(lambda:payload(core,'stdout')==b'parent ready')
        run.stop()
        eventually(lambda:all(not process_alive(pid) for pid in pids))


def missing_program(directory):
    plugin=spec('input-program',dict(command=str(directory/'does-not-exist')))
    with Core(plugins=[plugin]) as core, Input(core,plugin) as run:
        report=run.wait(success=False)
        assert report['state']=='failed' and 'spawn' in report['error']


def core_loss(directory):
    source=directory/'follow.log'
    source.write_bytes(b'old')
    plugins = [spec('input-file',dict(path=str(source))),
               spec('input-program',dict(command=sys.executable,args=['-c','import time;time.sleep(60)']))]
    for plugin in plugins:
        with Core(plugins=[plugin]) as core, Input(core,plugin) as run:
            run.ready('following' if plugin['bin']=='input-file' else 'capturing')
            core.stop()
            code=run.process.wait(timeout=8)
            assert code!=0, 'Core loss must not look like successful user stop'
            assert 'Core' in run.log_path.read_text() or 'connection' in run.log_path.read_text()


def invalid_file(directory):
    candidates = [directory/'missing-file', directory]
    if os.name != 'nt':
        fifo = directory/'not-a-regular-file'
        os.mkfifo(fifo)
        candidates.append(fifo)
    for path in candidates:
        plugin=spec('input-file', dict(path=str(path),mode='static'))
        with Core(plugins=[plugin]) as core, Input(core,plugin) as run:
            run.wait(success=False)
            assert records(core)==[]


def tmux_live(directory):
    if os.name=='nt' or not shutil.which('tmux'):
        return 'skipped: tmux not available on this platform'
    # Only this private server/socket is ever touched.
    with tempfile.TemporaryDirectory(prefix='lp-tmux-',dir='/tmp') as short:
        server=Path(short)/'server.sock'
        control=directory/'tmux-control'
        producer=directory/'tmux-source.py'
        producer.write_text("import os,pathlib,sys,time\np=pathlib.Path(sys.argv[1]);seen=''\nprint('BEFORE_ATTACH',flush=True)\nwhile True:\n text=p.read_text() if p.exists() else ''\n if text!=seen:\n  seen=text;os.write(1,('TMUX_'+text+'\\n').encode())\n time.sleep(.01)\n")
        def tmux(*args):
            result=subprocess.run(['tmux','-S',str(server),*args],capture_output=True,text=True,timeout=6)
            assert result.returncode==0,(args,result.stdout,result.stderr)
            return result.stdout.strip()
        try:
            command=' '.join(map(shlex.quote,[sys.executable,'-u',str(producer),str(control)]))
            pane=tmux('new-session','-d','-P','-F','#{pane_id}','-s','acceptance',command)
            pane_pid=int(tmux('display-message','-p','-t',pane,'#{pane_pid}'))
            eventually(lambda:'BEFORE_ATTACH' in tmux('capture-pane','-p','-t',pane))
            # Exercise both tmux parsing and its shell command with real spaces
            # and apostrophes in the helper executable path.
            copied_binary=directory/"input program's copy"
            shutil.copy2(BIN/('input-program'+EXE),copied_binary)
            plugin=spec(str(copied_binary),dict(mode='tmux',tmux_target=pane,tmux_socket=str(server)))
            with Core(plugins=[plugin]) as core, Input(core,plugin) as run:
                report=run.ready('capturing')
                assert report['history_imported'] is False and payload(core)==b''
                control.write_text('AFTER')
                eventually(lambda:b'TMUX_AFTER' in payload(core,'terminal'))
                assert b'BEFORE_ATTACH' not in payload(core)
                run.stop()
                eventually(lambda:tmux('display-message','-p','-t',pane,'#{pane_pipe}')=='0')
                assert process_alive(pane_pid)
                control.write_text('STILL_RUNNING')
                eventually(lambda:'TMUX_STILL_RUNNING' in tmux('capture-pane','-p','-t',pane))
                assert b'STILL_RUNNING' not in payload(core)
            # An existing user's pipe must remain active and receive new output.
            other=directory/'existing-pipe.log'
            tmux('pipe-pane','-O','-t',pane,'cat > '+shlex.quote(str(other)))
            with Core(plugins=[plugin]) as core, Input(core,plugin) as run:
                report=run.wait(success=False)
                assert 'already has pipe-pane' in report['error']
                assert tmux('display-message','-p','-t',pane,'#{pane_pipe}')=='1'
                control.write_text('EXISTING_PIPE')
                eventually(lambda:other.exists() and b'TMUX_EXISTING_PIPE' in other.read_bytes())
            tmux('pipe-pane','-t',pane)
            # A competing pipe installed AFTER the initial query must survive.
            # The wrapper adds the race deterministically on this private server.
            raced=directory/'raced-pipe.log'
            wrapper_dir=directory/'bin';wrapper_dir.mkdir()
            wrapper=wrapper_dir/'tmux'
            real_tmux=shutil.which('tmux')
            wrapper.write_text('#!'+sys.executable+'\n'+
                'import subprocess,sys\n'+
                'args=sys.argv[1:]\n'+
                'r=subprocess.run(['+repr(real_tmux)+']+args,capture_output=True)\n'+
                "if 'display-message' in args and any('pane_id' in a for a in args):\n"+
                ' subprocess.run(['+repr(real_tmux)+',"-S",'+repr(str(server))+',"pipe-pane","-O","-t",'+repr(pane)+','+repr('cat > '+shlex.quote(str(raced)))+'],check=True)\n'+
                'sys.stdout.buffer.write(r.stdout);sys.stderr.buffer.write(r.stderr);sys.exit(r.returncode)\n')
            wrapper.chmod(0o700)
            with Core(plugins=[plugin]) as core, Input(core,plugin,dict(PATH=str(wrapper_dir)+os.pathsep+os.environ['PATH'])) as run:
                report=run.wait(success=False)
                assert 'concurrent replacement' in report['error'], report
                assert tmux('display-message','-p','-t',pane,'#{pane_pipe}')=='1'
                control.write_text('RACED_PIPE')
                eventually(lambda:raced.exists() and b'TMUX_RACED_PIPE' in raced.read_bytes())
            tmux('pipe-pane','-t',pane)
            # Replacing our pipe after attachment must not let our cleanup close
            # that later replacement pipe (the helper/socket owns its own life).
            later=directory/'later-pipe.log'
            with Core(plugins=[plugin]) as core, Input(core,plugin) as run:
                run.ready('capturing')
                tmux('pipe-pane','-O','-t',pane,'cat > '+shlex.quote(str(later)))
                run.wait()
                assert tmux('display-message','-p','-t',pane,'#{pane_pipe}')=='1'
                control.write_text('LATER_PIPE')
                eventually(lambda:later.exists() and b'TMUX_LATER_PIPE' in later.read_bytes())
                assert process_alive(pane_pid)
        finally:
            subprocess.run(['tmux','-S',str(server),'kill-server'],capture_output=True,timeout=5)


def main():
    cases=[static_binary,static_empty,follow,from_start,program_channels,nonzero,stop_tree,missing_program,invalid_file,core_loss,tmux_live]
    for case in cases:
        with tempfile.TemporaryDirectory(prefix='log-print-input-test-') as temporary:
            result=case(Path(temporary))
        RESULTS.append(dict(case=case.__name__,result=result or 'passed'))
        print(json.dumps(RESULTS[-1]),flush=True)
    print(json.dumps(dict(suite='inputs',cases=RESULTS,passed=sum(x['result']=='passed' for x in RESULTS),skipped=sum(x['result']!='passed' for x in RESULTS))))


if __name__=='__main__': main()
