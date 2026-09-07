#!/usr/bin/env python3
"""Bounded, actual-Core integration checks for official byte plugins."""
import json
import os
from pathlib import Path
import select
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
import uuid
from unittest import SkipTest

ROOT = Path(__file__).resolve().parents[2]
BIN = ROOT / 'target' / 'debug'
SUFFIX = '.exe' if os.name == 'nt' else ''
RESULTS = []


def wait(check, message, seconds=8):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        value = check()
        if value:
            return value
        time.sleep(0.02)
    raise AssertionError(message)


def content(path):
    return path.read_bytes() if path.exists() else b''


def native_replace_fixture(directory):
    """Rust's Windows rename supports replacing an open, DELETE-shared target.

    Python 3.12 os.replace only calls MoveFileExW and rejects that target even
    when every reader shares DELETE. Rust falls back to FileRenameInfoEx with
    REPLACE_IF_EXISTS | POSIX_SEMANTICS. Keep the source reader open throughout;
    this fixture performs a real replacement, not a delete/truncate workaround.
    References: https://bugs.python.org/issue46003,
    rust-lang/rust issue123985 and std/sys/fs/windows.rs rename implementation.
    """
    compiler=shutil.which('rustc')
    assert compiler,'rustc is required for the Windows atomic replacement fixture'
    source=directory/'replace_fixture.rs';binary=directory/('replace_fixture'+SUFFIX)
    source.write_text('''fn main() -> std::io::Result<()> {
    let mut args = std::env::args_os().skip(1);
    std::fs::rename(args.next().expect("source"), args.next().expect("target"))
}
''')
    result=subprocess.run([compiler,str(source),'-o',str(binary)],capture_output=True,timeout=60)
    assert result.returncode==0,result.stderr.decode(errors='replace')
    def replace(source,target):
        result=subprocess.run([str(binary),str(source),str(target)],capture_output=True,timeout=8)
        assert result.returncode==0,result.stderr.decode(errors='replace')
    return replace


def plugin(name, binary, config, writes=(), reads=(), parents=()):
    return {'id': name, 'bin': binary, 'reads': list(reads),
            'streams': [{'id': s, 'parents': list(parents)} for s in writes],
            'config': config}


class Runtime:
    def __init__(self, directory, specs):
        self.dir = directory
        self.specs = {s['id']: s for s in specs}
        self.token = uuid.uuid4().hex
        self.children = {}
        self.logs = []
        self.tokens = {s['id']: uuid.uuid4().hex for s in specs}
        config = {'config': {'plugins': specs}, 'admin_token': self.token,
                  'plugin_tokens': self.tokens}
        path = directory / 'runtime.json'
        path.write_text(json.dumps(config))
        self.ready = directory / 'ready.json'
        log = open(directory / 'core.stderr', 'wb'); self.logs.append(log)
        self.core = subprocess.Popen([str(BIN / ('log-print-core'+SUFFIX)),
            '--runtime-config', str(path), '--ready-file', str(self.ready)],
            stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=log)
        wait(lambda: self.ready.exists(), 'Core did not become ready')
        self.address = json.loads(self.ready.read_text())['address']

    def rpc(self, operation, args=None):
        host, port = self.address.rsplit(':', 1)
        with socket.create_connection((host, int(port)), timeout=12) as sock:
            stream = sock.makefile('rwb')
            stream.write(json.dumps({'protocol': 'log-print/1', 'plugin': '__admin__',
                'token': self.token, 'events': False}).encode()+b'\n'); stream.flush()
            welcome = json.loads(stream.readline())
            assert not welcome.get('error'), welcome
            stream.write(json.dumps({'id': 1, 'op': operation, 'args': args or {}}).encode()+b'\n'); stream.flush()
            result = json.loads(stream.readline())
            assert not result.get('error'), result
            return result['result']

    def start(self, name, address=None):
        spec = self.specs[name]
        env = dict(os.environ, LOG_PRINT_CORE=address or self.address, LOG_PRINT_PLUGIN=name,
                   LOG_PRINT_TOKEN=self.tokens[name], LOG_PRINT_CONFIG=json.dumps(spec['config']))
        log = open(self.dir / (name+'.stderr'), 'wb'); self.logs.append(log)
        process = subprocess.Popen([str(BIN / (spec['bin']+SUFFIX))], env=env,
            stdout=subprocess.DEVNULL, stderr=log)
        self.children[name] = process
        return process

    def stop(self, name):
        p = self.children[name]
        if p.poll() is None:
            self.rpc('control', {'target': name, 'method': 'shutdown'})
        status = p.wait(timeout=6)
        assert status == 0, (name, status, (self.dir/(name+'.stderr')).read_text())

    def close(self):
        for p in self.children.values():
            if p.poll() is None:
                p.terminate()
        for p in self.children.values():
            try: p.wait(timeout=6)
            except subprocess.TimeoutExpired: p.kill(); p.wait(timeout=3)
        self.core.stdin.close()
        try: self.core.wait(timeout=5)
        except subprocess.TimeoutExpired: self.core.kill(); self.core.wait(timeout=3)
        for log in self.logs: log.close()


def run_case(name, fn):
    with tempfile.TemporaryDirectory(prefix='log-print-io-') as temporary:
        directory = Path(temporary)
        runtime = None
        try:
            runtime = fn(directory)
            RESULTS.append({'case': name, 'status': 'passed'})
        except SkipTest as e:
            RESULTS.append({'case':name,'status':'skipped','reason':str(e)})
        except Exception:
            for path in directory.glob('*.stderr'):
                print(path.name+': '+path.read_text(errors='replace'), file=sys.stderr)
            raise
        finally:
            if runtime: runtime.close()


def with_runtime(directory, specs, test):
    r = Runtime(directory, specs)
    try: test(r)
    except BaseException: r.close(); raise
    return r


def program(directory):
    out, err = directory/'out', directory/'err'
    source = directory/'source.py'
    source.write_text("import os\nos.write(1,b'A\\x00\\xffwithout-newline')\nos.write(2,b'error-without-newline')\nassert 'LOG_PRINT_TOKEN' not in os.environ\n")
    specs = [plugin('source','input-program',{'command':sys.executable,'args':['-u',str(source)],'stdout_stream':'out','stderr_stream':'err'},['out','err']),
             plugin('raw','output-raw',{'streams':['out','err'],'paths':{'out':str(out),'err':str(err)}},reads=['out','err'])]
    def test(r):
        r.start('raw'); p=r.start('source')
        assert p.wait(timeout=8)==0
        wait(lambda:content(out)==b'A\x00\xffwithout-newline' and content(err)==b'error-without-newline','separate raw bytes mismatch')
        r.stop('raw')
    return with_runtime(directory,specs,test)


def file_follow(directory):
    source,out=directory/'source.log',directory/'out';source.write_bytes(b'initial')
    replace=native_replace_fixture(directory) if os.name=='nt' else lambda a,b:a.replace(b)
    specs=[plugin('source','input-file',{'path':str(source),'stream':'file','from_start':True,'poll_ms':10},['file']),plugin('raw','output-raw',{'streams':['file'],'path':str(out)},reads=['file'])]
    def test(r):
        r.start('raw');r.start('source')
        wait(lambda:content(out)==b'initial','initial file')
        with source.open('ab') as f:f.write(b'++')
        wait(lambda:content(out)==b'initial++','append file')
        source.write_bytes(b'x')
        wait(lambda:content(out)==b'initial++x','truncate file')
        new=directory/'new';new.write_bytes(b'replacement');replace(new,source)
        wait(lambda:content(out)==b'initial++xreplacement','replace file')
        report=next(p['report'] for p in r.rpc('status')['plugins'] if p['id']=='source')
        assert report['reason']=='replaced' and report['segment']==2,report
        r.rpc('control',{'target':'source','method':'config.patch','args':{'poll_ms':20}})
        r.stop('source');r.stop('raw')
    return with_runtime(directory,specs,test)


def file_tail_registration(directory):
    """Append while registration is held: tail must already have its start offset."""
    source,out=directory/'source.log',directory/'out'
    source.write_bytes(b'old bytes must be skipped')
    added=b'new during registration\x00\xff'
    specs=[plugin('source','input-file',{'path':str(source),'stream':'file','poll_ms':10},['file']),
           plugin('raw','output-raw',{'streams':['file'],'path':str(out)},reads=['file'])]
    def test(r):
        hello_received=threading.Event();release=threading.Event();errors=[]
        with socket.socket() as listener:
            listener.bind(('127.0.0.1',0));listener.listen(1);listener.settimeout(8)
            address=f'127.0.0.1:{listener.getsockname()[1]}'
            def proxy():
                try:
                    incoming,_=listener.accept()
                    with incoming:
                        incoming.settimeout(8)
                        hello=b''
                        while not hello.endswith(b'\n'):
                            byte=incoming.recv(1)
                            if not byte:raise RuntimeError('source disconnected before handshake')
                            hello+=byte
                            if len(hello)>1048576:raise RuntimeError('handshake exceeds limit')
                        hello_received.set()
                        if not release.wait(8):raise RuntimeError('registration gate timed out')
                        host,port=r.address.rsplit(':',1)
                        with socket.create_connection((host,int(port)),timeout=8) as core:
                            core.sendall(hello)
                            while True:
                                readable,_,_=select.select([incoming,core],[],[],8)
                                if not readable:raise RuntimeError('proxy idle timeout')
                                for reader in readable:
                                    data=reader.recv(65536)
                                    if not data:return
                                    (core if reader is incoming else incoming).sendall(data)
                except Exception as error:errors.append(str(error))
            worker=threading.Thread(target=proxy,daemon=True);worker.start()
            try:
                r.start('raw');r.start('source',address)
                assert hello_received.wait(8),errors
                with source.open('ab') as f:f.write(added)
                release.set()
                wait(lambda:content(out)==added,'tail skipped bytes appended while registering')
                r.rpc('control',{'target':'source','method':'config.patch','args':{'poll_ms':20}})
                with source.open('ab') as f:f.write(b':after-patch')
                wait(lambda:content(out)==added+b':after-patch','dynamic poll patch lost append')
                r.stop('source');r.stop('raw')
            finally:
                release.set()
                if r.children.get('source') and r.children['source'].poll() is None:
                    r.children['source'].terminate();r.children['source'].wait(timeout=6)
                worker.join(timeout=9)
            assert not worker.is_alive(),'registration proxy did not stop'
            assert not errors,errors
    return with_runtime(directory,specs,test)


def replay_transform(directory):
    source=directory/'source.log';payload='temp=中\nnext=2'.encode();source.write_bytes(payload)
    raw,derived=directory/'raw',directory/'derived'
    specs=[plugin('source','input-replay',{'path':str(source),'stream':'source','timestamp':'none','interval_ms':0,'chunk_bytes':1},['source']),
        plugin('transform','output-transform',{'streams':['source'],'output_stream':'derived','input_encoding':'utf-8','split_lines':True,'replace':[{'pattern':'temp=','with':'T='}]},['derived'],['source'],['source']),
        plugin('raw','output-raw',{'streams':['source','derived'],'paths':{'source':str(raw),'derived':str(derived)}},reads=['source','derived'])]
    def test(r):
        r.start('raw');r.start('transform');p=r.start('source');assert p.wait(timeout=8)==0
        wait(lambda:content(raw)==payload,'transparent replay differs')
        wait(lambda:content(derived)=='T=中\n'.encode(),'cross-chunk decoded line differs')
        r.stop('transform')
        wait(lambda:content(derived)=='T=中\nnext=2'.encode(),'shutdown partial line flush')
        r.stop('raw')
    return with_runtime(directory,specs,test)


def multi_parent(directory):
    a,b=directory/'a.log',directory/'b.log';a.write_bytes(b'A');b.write_bytes(b'B')
    out=directory/'out'
    specs=[plugin('a','input-replay',{'path':str(a),'stream':'a','timestamp':'none'},['a']),
        plugin('b','input-replay',{'path':str(b),'stream':'b','timestamp':'none'},['b']),
        plugin('transform','output-transform',{'streams':['a','b'],'output_stream':'joined','input_encoding':'raw'},['joined'],['a','b'],['a','b']),
        plugin('raw','output-raw',{'streams':['joined'],'path':str(out)},reads=['joined'])]
    def test(r):
        r.start('raw');r.start('transform');assert r.start('a').wait(timeout=8)==0
        assert r.start('b').wait(timeout=8)==0
        wait(lambda:len(content(out))==2,'multi-parent pending outputs did not drain')
        assert sorted(content(out))==list(b'AB'),content(out)
        records=r.rpc('read',{'stream':'joined','from':1,'limit':16})['records']
        assert all(set(item['upstream'])=={'a','b'} for item in records),records
        effective=r.rpc('control',{'target':'transform','method':'config.get'})
        assert effective['effective']['max_pending_bytes']==65536,effective
        assert effective['origins']['max_pending_bytes']=='built_in_default',effective
        r.stop('transform');r.stop('raw')
    return with_runtime(directory,specs,test)


def source_tree(directory):
    out=directory/'out';source=directory/'source.py'
    source.write_text("import subprocess,sys,time\np=subprocess.Popen([sys.executable,'-c','import time;time.sleep(120)'])\nprint(p.pid,flush=True)\ntime.sleep(120)\n")
    specs=[plugin('source','input-program',{'command':sys.executable,'args':['-u',str(source)],'stdout_stream':'out','stderr_stream':'err'},['out','err']),plugin('raw','output-raw',{'streams':['out'],'path':str(out)},reads=['out'])]
    def test(r):
        r.start('raw');r.start('source');wait(lambda:b'\n'in content(out),'child PID not emitted')
        child=int(content(out));r.stop('source')
        def gone():
            if os.name=='nt':
                status=subprocess.run(['tasklist','/FI',f'PID eq {child}','/FO','CSV','/NH'],capture_output=True,text=True).stdout
                return str(child) not in status
            status=subprocess.run(['ps','-o','stat=','-p',str(child)],capture_output=True,text=True).stdout.strip()
            return not status or status.startswith('Z')
        wait(gone,'source descendant survived owned process-group shutdown')
        r.stop('raw')
    return with_runtime(directory,specs,test)


def serial_pty(directory):
    import tty
    master,slave=os.openpty();tty.setraw(slave);port=os.ttyname(slave);out=directory/'out'
    specs=[plugin('source','input-serial',{'port':port,'stream':'serial'},['serial']),plugin('raw','output-raw',{'streams':['serial'],'path':str(out)},reads=['serial'])]
    def test(r):
        r.start('raw');r.start('source')
        def reading():
            p=r.children['source']
            if p.poll() is not None:
                reason=(directory/'source.stderr').read_text()
                if sys.platform=='darwin' and 'Not a typewriter' in reason:
                    raise SkipTest('macOS PTY rejects serial driver configuration with ENOTTY; physical serial hardware not tested')
                raise AssertionError(reason)
            return any(p['id']=='source' and (p.get('report') or {}).get('state')=='reading' for p in r.rpc('status')['plugins'])
        wait(reading,'serial not reading')
        os.write(master,b'pty\x00\xff-no-newline')
        wait(lambda:content(out)==b'pty\x00\xff-no-newline','PTY serial bytes differ')
        r.stop('source');r.stop('raw')
    try:return with_runtime(directory,specs,test)
    finally:os.close(master);os.close(slave)


if __name__=='__main__':
    for name,case in [('program_raw',program),('file_follow',file_follow),('file_tail_registration',file_tail_registration),('replay_transform',replay_transform),('multi_parent_transform',multi_parent),('source_tree_cleanup',source_tree)]:run_case(name,case)
    if os.name=='posix':run_case('serial_pty_not_physical_hardware',serial_pty)
    print(json.dumps({'platform':sys.platform,'python':sys.version.split()[0],'cases':RESULTS,'real_hardware':'not_tested'},indent=2))
