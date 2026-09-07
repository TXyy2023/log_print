#!/usr/bin/env python3
"""Real-process latency/resource benchmark; no synthetic Core timing claims.

Use an isolated Python environment with psutil, and prebuild `cargo build --release
--workspace`. Unix is currently required for the PTY comparison. Per-scenario
artifacts omit runtime tokens and retain raw timing/resource samples.
"""
import argparse
import collections
import gzip
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import select
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = Path(__file__).resolve()
LINE_BYTES = 96


def line(index):
    value = 50.0 + 20.0 * math.sin(index / 17)
    return f'{index:08d} value={value:08.3f} '.encode().ljust(LINE_BYTES-1, b'x') + b'\n'


def source(args):
    directory = Path(args.source_dir)
    (directory/'source.ready').write_text(str(os.getpid()))
    deadline = time.monotonic() + 30
    while not (directory/'gate').exists():
        if time.monotonic() > deadline:
            raise RuntimeError('benchmark gate timed out')
        time.sleep(0.001)
    begin = time.perf_counter_ns()
    timings = []
    count = round(args.duration * args.rate)
    for index in range(count):
        target = begin + round(index * 1e9 / args.rate)
        delay = (target - time.perf_counter_ns()) / 1e9
        if delay > 0:
            time.sleep(delay)
        payload = line(index)
        before = time.perf_counter_ns()
        written = os.write(1, payload)
        after = time.perf_counter_ns()
        if written != len(payload):
            raise RuntimeError('short source os.write')
        timings.append([index, before, after])
    data = {'pid': os.getpid(), 'rate_target': args.rate,
            'duration_target': args.duration, 'records': timings}
    temporary = directory/'source.json.pending'
    temporary.write_text(json.dumps(data)); temporary.replace(directory/'source.json')
    time.sleep(1.0)  # Keep the source alive for the last resource sample; outside measured writes.


def extract():
    pending = b''
    while True:
        chunk = os.read(0, 65536)
        if not chunk:
            break
        pending += chunk
        while b'\n' in pending:
            record, pending = pending.split(b'\n', 1)
            value = record.split(b'value=', 1)[1].split(b' ', 1)[0]
            os.write(1, value+b'\n')
        if len(pending)>LINE_BYTES:
            raise RuntimeError('extractor framing exceeds one benchmark record')
    if pending:
        raise RuntimeError('extractor received incomplete benchmark record')


def wait(check, message, timeout=15):
    end = time.monotonic()+timeout
    while time.monotonic()<end:
        value=check()
        if value:
            return value
        time.sleep(0.01)
    raise RuntimeError(message)


class Observation:
    def __init__(self, fd, count):
        self.fd, self.count = fd,count
        self.observed=[];self.payload=bytearray();self.error=None
        self.stop=threading.Event()
        self.thread=threading.Thread(target=self.run,daemon=True);self.thread.start()

    def run(self):
        pending=b''
        try:
            while len(self.observed)<self.count and not self.stop.is_set():
                if not select.select([self.fd],[],[],0.1)[0]:continue
                chunk=os.read(self.fd,65536)
                now=time.perf_counter_ns()
                if not chunk:break
                self.payload.extend(chunk)
                if len(self.payload)>self.count*LINE_BYTES+65536:
                    raise RuntimeError('output exceeded expected byte bound')
                pending+=chunk
                while b'\n'in pending:
                    record,pending=pending.split(b'\n',1)
                    index=len(self.observed)
                    if record+b'\n'!=line(index):raise RuntimeError(f'raw bytes/order mismatch at record {index}')
                    self.observed.append(now)
            if pending:raise RuntimeError('partial output record')
        except Exception as e:self.error=str(e)

    def done(self):
        if self.error:raise RuntimeError(self.error)
        return len(self.observed)==self.count

    def close(self):
        self.stop.set();self.thread.join(timeout=2)


class Terminal:
    def __init__(self):
        import fcntl,termios
        self.master,self.slave=os.openpty()
        fcntl.ioctl(self.slave,termios.TIOCSWINSZ,struct.pack('HHHH',30,100,0,0))
        self.path=os.ttyname(self.slave);self.bytes=0;self.stop=threading.Event()
        self.thread=threading.Thread(target=self.drain,daemon=True);self.thread.start()

    def drain(self):
        while not self.stop.is_set():
            if not select.select([self.master],[],[],0.1)[0]:continue
            try:data=os.read(self.master,65536)
            except OSError:break
            if not data:break
            self.bytes+=len(data)

    def close(self):
        self.stop.set();self.thread.join(timeout=2)
        os.close(self.master);os.close(self.slave)


class Sampler:
    def __init__(self,roots):
        import psutil
        self.psutil=psutil;self.roots=roots;self.rows=[];self.baseline={};self.last={}
        self.stop=threading.Event();self.started=time.perf_counter_ns()
        self.self_before=sum(psutil.Process().cpu_times()[:2])
        self.sample(initial=True)
        self.thread=threading.Thread(target=self.loop,daemon=True);self.thread.start()

    def sample(self,initial=False):
        ps=self.psutil;processes={}
        for pid in self.roots:
            try:
                p=ps.Process(pid);processes[p.pid]=p
                for child in p.children(recursive=True):processes[child.pid]=child
            except (ps.NoSuchProcess,ps.AccessDenied):pass
        row={'time_ns':time.perf_counter_ns(),'host_cpu_percent':ps.cpu_percent(),'processes':[]}
        for p in processes.values():
            try:
                with p.oneshot():
                    cmd=p.cmdline();name=Path(cmd[0]).name if cmd else p.name()
                    if '--source'in cmd:name='source-python'
                    elif '--extract'in cmd:name='extractor-python'
                    cpu=sum(p.cpu_times()[:2]);rss=p.memory_info().rss
                    key=f'{p.pid}:{p.create_time()}'
                    item={'pid':p.pid,'role':name,'cpu_seconds':cpu,'rss_bytes':rss}
                    try:
                        io=p.io_counters();item['read_bytes']=io.read_bytes;item['write_bytes']=io.write_bytes
                    except (AttributeError,ps.Error):item['read_bytes']=None;item['write_bytes']=None
                    if initial:self.baseline[key]=cpu
                    self.last[key]=item
                    row['processes'].append(item)
            except (ps.NoSuchProcess,ps.AccessDenied):pass
        row['tree_rss_bytes']=sum(p['rss_bytes'] for p in row['processes'])
        self.rows.append(row)

    def loop(self):
        while not self.stop.wait(0.1):self.sample()

    def finish(self):
        self.stop.set();self.thread.join(timeout=3);self.sample()
        duration=(self.rows[-1]['time_ns']-self.started)/1e9
        by_role={}
        for key,p in self.last.items():
            part=by_role.setdefault(p['role'],{'cpu_seconds':0,'peak_rss_bytes':0})
            part['cpu_seconds']+=max(0,p['cpu_seconds']-self.baseline.get(key,0))
        for row in self.rows:
            totals=collections.Counter()
            for p in row['processes']:totals[p['role']]+=p['rss_bytes']
            for role,rss in totals.items():by_role[role]['peak_rss_bytes']=max(by_role[role]['peak_rss_bytes'],rss)
        for part in by_role.values():part['cpu_percent_one_core']=100*part['cpu_seconds']/duration
        cpu=sum(v['cpu_seconds'] for v in by_role.values())
        return {'duration_s':duration,'tree_cpu_seconds':cpu,'tree_cpu_percent_one_core':100*cpu/duration,
                'tree_peak_rss_bytes':max(r['tree_rss_bytes'] for r in self.rows),'components':by_role,
                'harness_cpu_seconds':sum(self.psutil.Process().cpu_times()[:2])-self.self_before,
                'host_cpu_percent_mean':sum(r['host_cpu_percent'] for r in self.rows)/len(self.rows),
                'sample_interval_s':0.1,'sample_count':len(self.rows),'raw_samples':self.rows}


def sample_process(args):
    directory=Path(args.sample_dir)
    sampler=Sampler([int(x) for x in args.roots.split(',')])
    (directory/'sampler.ready').write_text(str(os.getpid()))
    while not (directory/'sampler.stop').exists():time.sleep(0.01)
    result=sampler.finish()
    result['sampler_cpu_seconds']=sum(__import__('psutil').Process().cpu_times()[:2])
    (directory/'resources.json').write_text(json.dumps(result))


class SamplerProcess:
    # Keep psutil calls out of the output observer's interpreter/GIL.
    def __init__(self,roots,directory):
        self.directory=directory
        self.before=sum(__import__('psutil').Process().cpu_times()[:2])
        self.process=subprocess.Popen([sys.executable,str(SCRIPT),'--sample','--roots',','.join(map(str,roots)),'--sample-dir',str(directory)],stdout=subprocess.DEVNULL)
        wait(lambda:(directory/'sampler.ready').exists(),'sampler not ready')
    def finish(self):
        (self.directory/'sampler.stop').write_text('stop')
        self.process.wait(timeout=10)
        if self.process.returncode:raise RuntimeError('resource sampler failed')
        data=json.loads((self.directory/'resources.json').read_text())
        data['harness_cpu_seconds']=sum(__import__('psutil').Process().cpu_times()[:2])-self.before
        return data


def rpc(address,token,operation,args):
    host,port=address.rsplit(':',1)
    with socket.create_connection((host,int(port)),timeout=15)as sock:
        f=sock.makefile('rwb')
        f.write(json.dumps({'protocol':'log-print/1','plugin':'__admin__','token':token}).encode()+b'\n');f.flush()
        welcome=json.loads(f.readline());assert not welcome.get('error'),welcome
        f.write(json.dumps({'id':1,'op':operation,'args':args}).encode()+b'\n');f.flush()
        answer=json.loads(f.readline());assert not answer.get('error'),answer
        return answer['result']


def percentile(values,p):
    values=sorted(values)
    return values[min(len(values)-1,max(0,math.ceil(len(values)*p/100)-1))]


def summarize_latencies(source_times,observed):
    before=[(o-r[1])/1e6 for r,o in zip(source_times,observed)]
    after=[(o-r[2])/1e6 for r,o in zip(source_times,observed)]
    widths=[(r[2]-r[1])/1e6 for r in source_times]
    def stats(xs):return {f'p{p}_ms':percentile(xs,p) for p in [50,95,99]}|{'max_ms':max(xs)}
    return {'observer_minus_source_write_before':stats(before),
            'observer_minus_source_write_after':stats(after),
            'source_write_bracket':stats(widths),'negative_after_count':sum(v<0 for v in after)}


def run_case(args,scenario):
    if os.name!='posix':raise RuntimeError('benchmark harness currently requires Unix pipes/PTYS; no Windows measurement claim')
    is_app=scenario.startswith('log_print_');mode=scenario.split('_')[-1]
    rate=args.plot_rate if mode=='plot' else args.rate
    count=round(rate*args.duration)
    if count<10 or count>200000:raise RuntimeError('benchmark count must be 10..200000')
    output=Path(args.output);output.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='log-print-measure-')as tmp:
        directory=Path(tmp);processes=[];fds=[];logs=[];terminal=None;observer=None;sampler=None
        source_command=[sys.executable,str(SCRIPT),'--source','--source-dir',tmp,'--rate',str(rate),'--duration',str(args.duration)]
        def launch(command,**kwargs):
            log=open(directory/f'process-{len(processes)}.stderr','wb');logs.append(log)
            p=subprocess.Popen(command,stderr=log,**kwargs);processes.append(p);return p
        started=time.perf_counter_ns()
        extra={}
        try:
            if mode=='plot':terminal=Terminal()
            if is_app:
                bins=Path(args.bin_dir).resolve()
                specs=[{'id':'raw','bin':str(bins/'output-raw'),'reads':['bench'],
                        'config':{'streams':['bench'],'from':1}},
                       {'id':'source','bin':str(bins/'input-program'),'streams':[{'id':'bench'},{'id':'bench.err'}],
                        'save':{'enabled':mode=='save','directory':str(directory/'saved')},
                        'config':{'command':source_command[0],'args':source_command[1:],'stdout_stream':'bench','stderr_stream':'bench.err','chunk_bytes':4096}}]
                if mode=='plot':specs.append({'id':'tui','bin':str(bins/'output-tui'),'reads':['bench'],
                    'config':{'streams':['bench'],'tty':terminal.path,'sessions':[{'id':'default','refresh_ms':100,'max_points':2048,
                    'series':[{'name':'value','stream':'bench','pattern':r'value=(?P<value>[-+0-9.]+)'}]}]}})
                config={'plugins':specs}
                path=directory/'config.json';path.write_text(json.dumps(config));state=directory/'state.json'
                main=launch([str(bins/'log-print'),'--state',str(state),'run','--config',str(path)],stdout=subprocess.PIPE)
                observer=Observation(main.stdout.fileno(),count)
                def ready():
                    if main.poll() is not None:raise RuntimeError(f'supervisor exited {main.returncode}')
                    if state.exists():
                        try:
                            data=json.loads(state.read_text())
                            if 'core_address'in data:return data
                        except json.JSONDecodeError:pass
                instance=wait(ready,'supervisor not ready',30)
                roots=[main.pid]
                extra['configuration']=json.loads(json.dumps(config).replace(tmp,'<temporary>'))
                extra['software_version']='1.0.0'
                extra['build_profile']='release' if bins.name=='release' else bins.name
                extra['source_state']='uncommitted working tree; historical git HEAD is not the tested version; binary SHA256 identifies tested artifact'
                extra['rustc_version']=subprocess.check_output(['rustc','--version'],text=True).strip()
                extra['binary_sha256']={name:hashlib.sha256((bins/name).read_bytes()).hexdigest() for name in ['log-print','log-print-core','input-program','output-raw']+(['output-tui']if mode=='plot'else[])}
            else:
                source_p=launch(source_command,stdout=subprocess.PIPE)
                if mode=='plot':
                    fifo=directory/'latency.pipe';os.mkfifo(fifo)
                    fd=os.open(fifo,os.O_RDWR);fds.append(fd);observer=Observation(fd,count)
                    tee=launch([args.tee,str(fifo)],stdin=source_p.stdout,stdout=subprocess.PIPE)
                    extractor=launch([sys.executable,str(SCRIPT),'--extract'],stdin=tee.stdout,stdout=subprocess.PIPE)
                    plot=launch([str(Path(args.ttyplot).resolve()),'-t','log_print equivalent scalar input','-b'],stdin=extractor.stdout,stdout=terminal.slave,env=dict(os.environ,TERM='xterm-256color'))
                    source_p.stdout.close();tee.stdout.close();extractor.stdout.close()
                    roots=[source_p.pid,tee.pid,extractor.pid,plot.pid]
                    extra['ttyplot_version']=subprocess.check_output([args.ttyplot,'-v'],text=True).strip()
                    extra['ttyplot_sha256']=hashlib.sha256(Path(args.ttyplot).read_bytes()).hexdigest()
                else:
                    command=[args.tee]+([str(directory/'tee-save.raw')]if mode=='save'else[])
                    tee=launch(command,stdin=source_p.stdout,stdout=subprocess.PIPE);source_p.stdout.close()
                    observer=Observation(tee.stdout.fileno(),count);roots=[source_p.pid,tee.pid]
                extra['tee_path']=args.tee
                extra['tee_sha256']=hashlib.sha256(Path(args.tee).read_bytes()).hexdigest()
            wait(lambda:(directory/'source.ready').exists(),'source not ready')
            if terminal:wait(lambda:terminal.bytes>0,'plot did not render terminal output')
            sampler=SamplerProcess(roots,directory)
            (directory/'gate').write_text('start')
            wait(observer.done,'output incomplete or too slow',args.duration+30)
            wait(lambda:(directory/'source.json').exists(),'source timing metadata missing')
            resources=sampler.finish();sampler=None
            source_data=json.loads((directory/'source.json').read_text())
            times=source_data['records'];assert len(times)==count
            if mode=='save' and not is_app:assert (directory/'tee-save.raw').read_bytes()==observer.payload
            if is_app:
                status=rpc(instance['core_address'],instance['token'],'status',{})
                extra['stream_status']=status['streams']
                if mode=='plot':extra['plot_sessions']=rpc(instance['core_address'],instance['token'],'control',{'target':'tui','method':'sessions','args':{}})
                if mode=='save':
                    saved=next(s for s in status['streams'] if s['id']=='bench')
                    assert saved['save']['enabled'] and saved['blocked'] is None,saved
                    cursor=1;history=bytearray();saved_records=0
                    while cursor<=saved['head']:
                        page=rpc(instance['core_address'],instance['token'],'read',{'stream':'bench','from':cursor,'limit':64})
                        assert not page['gap'],page
                        if page['next']<=cursor:raise RuntimeError('saved readback made no progress')
                        for record in page['records']:history.extend(record['payload']);saved_records+=1
                        cursor=page['next']
                    assert history==observer.payload,'saved historical byte readback differs'
                    extra['saved_readback_exact']=True;extra['saved_records']=saved_records
                    extra['saved_file_bytes']=sum(p.stat().st_size for p in (directory/'saved').rglob('*')if p.is_file())
            if terminal:extra['terminal']={'columns':100,'rows':30,'rendered_bytes':terminal.bytes,'external_emulator':'not_running','render_sink':'drained PTY'}
            latency=summarize_latencies(times,observer.observed)
            actual=(times[-1][2]-times[0][1])/1e9
            raw_path=output/(scenario+'.latency.csv.gz')
            with gzip.open(raw_path,'wt')as f:
                f.write('id,source_write_before_ns,source_write_after_ns,observer_read_ns\n')
                for record,observed in zip(times,observer.observed):f.write(','.join(map(str,[*record,observed]))+'\n')
            samples=resources.pop('raw_samples')
            (output/(scenario+'.resources.json')).write_text(json.dumps(samples,indent=2))
            result={'scenario':scenario,'platform':platform.platform(),'machine':platform.machine(),
                'python':sys.version.split()[0],'psutil':__import__('psutil').__version__,
                'logical_cpus':os.cpu_count(),'physical_memory_bytes':__import__('psutil').virtual_memory().total,
                'cpu_model':subprocess.check_output(['sysctl','-n','machdep.cpu.brand_string'],text=True).strip() if sys.platform=='darwin' else platform.processor(),
                'created_at_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),
                'requested_duration_s':args.duration,'target_records_per_second':rate,'samples':count,
                'bytes_per_record':LINE_BYTES,'output_bytes':len(observer.payload),
                'payload_sha256':hashlib.sha256(observer.payload).hexdigest(),'exact_payload_and_order':True,
                'actual_source_write_interval_s':actual,'actual_source_bytes_per_second':count*LINE_BYTES/actual,
                'output_observation_window_s':(observer.observed[-1]-times[0][1])/1e9,
                'actual_output_bytes_per_second':count*LINE_BYTES/((observer.observed[-1]-times[0][1])/1e9),
                'setup_elapsed_s':(times[0][1]-started)/1e9,
                'latency':latency,'resources':resources,**extra}
            (output/(scenario+'.json')).write_text(json.dumps(result,indent=2))
            print(json.dumps({'scenario':scenario,'latency':latency,'resources':resources,'samples':count}),flush=True)
            return result
        finally:
            if sampler:sampler.finish()
            if observer:observer.close()
            for p in reversed(processes):
                if p.poll() is None:p.terminate()
            for p in reversed(processes):
                try:p.wait(timeout=8)
                except subprocess.TimeoutExpired:p.kill();p.wait(timeout=3)
            for log in logs:log.close()
            if sys.exc_info()[0]:
                for p in directory.glob('*.stderr'):print(p.name+': '+p.read_text(errors='replace'),file=sys.stderr)
            if terminal:terminal.close()
            for fd in fds:os.close(fd)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source',action='store_true');parser.add_argument('--source-dir')
    parser.add_argument('--extract',action='store_true')
    parser.add_argument('--sample',action='store_true');parser.add_argument('--sample-dir');parser.add_argument('--roots')
    parser.add_argument('--duration',type=float,default=10)
    parser.add_argument('--rate',type=int,default=1000)
    parser.add_argument('--plot-rate',type=int,default=1000)
    parser.add_argument('--bin-dir',default=str(ROOT/'target/release'))
    parser.add_argument('--tee',default='/usr/bin/tee')
    parser.add_argument('--ttyplot',default='ttyplot')
    parser.add_argument('--output',default=str(ROOT/'doc/benchmarks/local'))
    parser.add_argument('--scenario',choices=['log_print_raw','log_print_save','log_print_plot','tee_raw','tee_save','tee_plot','all'],default='all')
    args=parser.parse_args()
    if args.source:return source(args)
    if args.extract:return extract()
    if args.sample:return sample_process(args)
    if args.duration<1 or args.rate<1 or args.plot_rate<1:parser.error('positive rates and duration>=1 required')
    for scenario in (['tee_raw','log_print_raw','tee_save','log_print_save','tee_plot','log_print_plot']if args.scenario=='all'else[args.scenario]):run_case(args,scenario)


if __name__=='__main__':main()
