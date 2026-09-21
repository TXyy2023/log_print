"""Real-process helpers for log-print/2 acceptance; stdlib only."""
import contextlib
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[3]
BIN = ROOT / 'target' / os.environ.get('LOG_PRINT_PROFILE', 'debug')
EXE = '.exe' if os.name == 'nt' else ''


def eventually(check, timeout=8):
    end = time.monotonic() + timeout
    last = None
    while time.monotonic() < end:
        last = check()
        if last:
            return last
        time.sleep(.025)
    raise AssertionError(f'condition not reached within {timeout}s; last={last!r}')


class RemoteError(Exception):
    def __init__(self, fault):
        self.code = fault['code']
        super().__init__(f'{self.code}: {fault["message"]}')


class RPC:
    def __init__(self, address, plugin='__admin__', token='admin', transport='tcp', events=False, protocol='log-print/2'):
        self.transport = transport
        self.seq = 0
        self.file = None
        host, port = address.rsplit(':', 1)
        self.socket = socket.socket(socket.AF_INET, socket.SOCK_DGRAM if transport == 'udp' else socket.SOCK_STREAM)
        self.socket.settimeout(4)
        self.socket.connect((host, int(port)))
        if transport == 'tcp':
            self.file = self.socket.makefile('rb')
            self.socket.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        try:
            self.send({'protocol':protocol,'plugin':plugin,'token':token,'events':events})
            self.welcome = self.result(self.receive())
        except BaseException:
            self.close()
            raise

    def send(self, value):
        data = json.dumps(value, separators=(',', ':')).encode()
        if self.transport == 'tcp':
            self.socket.sendall(data + b'\n')
        else:
            self.socket.send(data)

    def receive(self):
        data = self.file.readline(1024*1024+1) if self.file else self.socket.recv(65536)
        if not data:
            raise EOFError('Core disconnected')
        assert len(data) <= 1024*1024
        return json.loads(data)

    @staticmethod
    def result(message):
        if message.get('error'):
            raise RemoteError(message['error'])
        return message.get('result')

    def call(self, op, **args):
        self.seq += 1
        self.send({'id':self.seq,'op':op,'args':args})
        message = self.receive()
        assert message['type'] == 'response' and message['id'] == self.seq, message
        return self.result(message)

    def publish(self, stream, payload, key='same-key', **extra):
        args = dict(stream=stream, payload=list(payload), key=key, **extra)
        if self.transport == 'udp':
            self.send({'id':0,'op':'publish','args':args})
            return None
        return self.call('publish', **args)

    def close(self):
        if self.transport == 'udp':
            with contextlib.suppress(OSError):
                self.send({'id':0,'op':'disconnect','args':{}})
        if self.file:
            self.file.close()
        self.socket.close()

    def __enter__(self):
        return self

    def __exit__(self, *args):
        self.close()


class Core:
    def __init__(self, transport='tcp', options=None, plugins=None):
        self.temp = tempfile.TemporaryDirectory(prefix='log-print-v2-core-')
        self.path = Path(self.temp.name)
        self.transport = transport
        self.plugins = plugins or [
            {'id':'a','role':'input','bin':'unused','streams':[{'id':'raw','description':'stream A'}]},
            {'id':'b','role':'input','bin':'unused','streams':[{'id':'other','description':'stream B'}]},
            {'id':'out','role':'output','bin':'unused','reads':['raw','other']},
            {'id':'other-out','role':'output','bin':'unused','reads':['raw']},
        ]
        config = {'core':{'transport':transport, **(options or {})}, 'plugins':self.plugins}
        self.runtime = self.path / 'runtime.json'
        self.runtime.write_text(json.dumps({'config':config,'admin_token':'admin','plugin_tokens':{p['id']:p['id'] for p in self.plugins}}))
        self.log = open(self.path / 'core.log', 'ab')
        self.process = None
        self.address = None

    def start(self):
        ready = self.path / 'ready.json'
        ready.unlink(missing_ok=True)
        self.process = subprocess.Popen([str(BIN / ('log-print-core'+EXE)), '--runtime-config',str(self.runtime),'--ready-file',str(ready)],stdin=subprocess.PIPE,stdout=self.log,stderr=self.log)
        def check():
            if self.process.poll() is not None:
                raise AssertionError((self.path/'core.log').read_text())
            try:
                return json.loads(ready.read_text())
            except (OSError,ValueError):
                return None
        self.address = eventually(check)['address']
        return self

    def rpc(self, plugin='__admin__', **kwargs):
        return RPC(self.address,plugin, 'admin' if plugin=='__admin__' else plugin, self.transport, **kwargs)

    def streams(self):
        with self.rpc() as admin:
            return admin.call('status')['streams']

    def stream(self, owner):
        return next(s['id'] for s in self.streams() if s['owner']==owner)

    def stop(self):
        if self.process and self.process.poll() is None:
            self.process.stdin.close()
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=5)
                raise AssertionError('Core failed to exit on parent stdin EOF')
        self.process = None

    def __enter__(self):
        return self.start()

    def __exit__(self,*args):
        self.stop()
        self.log.close()
        self.temp.cleanup()


class App:
    def __init__(self, config):
        self.temp = tempfile.TemporaryDirectory(prefix='log-print-v2-app-')
        self.path = Path(self.temp.name)
        self.config = self.path / 'config.json'
        self.config.write_text(json.dumps(config))
        self.state = self.path / 'state.json'
        self.started = False

    def cli(self,*args,ok=True,timeout=45):
        r = subprocess.run([str(BIN/('log-print'+EXE)),'--state',str(self.state),*map(str,args)],cwd=ROOT,capture_output=True,text=True,timeout=timeout)
        if ok and r.returncode:
            logs = ''.join(p.read_text(errors='replace') for p in self.path.glob('*.log'))
            raise AssertionError(f'{args}: code={r.returncode}\n{r.stdout}\n{r.stderr}\n{logs[-8000:]}')
        return r

    def json(self,*args):
        return json.loads(self.cli(*args).stdout)

    def start(self):
        result = self.json('start','--config',self.config)
        self.started = True
        return result

    def __enter__(self):
        self.start()
        return self

    def __exit__(self,*args):
        if self.started and self.state.exists():
            self.cli('stop')
            assert not self.state.exists(), 'state remains after stop'
        self.temp.cleanup()
