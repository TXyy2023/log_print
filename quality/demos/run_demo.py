#!/usr/bin/env python3
"""Run and verify real CLI demos; emit JSON events consumed by record_demo.mjs.

All log payloads are explicit teaching fixtures. No external service is used.
"""
import argparse
from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import shlex
import sqlite3
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]
parser = argparse.ArgumentParser()
parser.add_argument('scenario', choices=['file-read', 'program-archive'])
parser.add_argument('--bin-dir', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--hold', type=float, default=0)
args = parser.parse_args()
BIN = args.bin_dir.resolve()
OUT = args.output.resolve()
OUT.mkdir(parents=True, exist_ok=True)
EVENTS = OUT / 'transcript.jsonl'
EVENTS.write_text('')
checks = []
owned = {}
completed = False
scenario = args.scenario
temporary = tempfile.TemporaryDirectory(prefix='log-print-demo-')
WORK = Path(temporary.name)
STATE = WORK / 'state.json'
CONFIG = WORK / 'config.json'


def redact(text):
    return (str(text).replace(str(BIN), '<bin-dir>').replace(str(WORK.resolve()), '<demo-dir>')
            .replace(str(WORK), '<demo-dir>').replace(str(ROOT), '<repo>').replace(sys.executable, '<python3>'))


def event(kind, **values):
    record = dict(kind=kind, at=time.time(), **values)
    record = json.loads(redact(json.dumps(record, ensure_ascii=False)))
    line = json.dumps(record, ensure_ascii=False)
    with EVENTS.open('a') as target:
        target.write(line + '\n')
    print(line, flush=True)


def pause():
    if args.hold:
        time.sleep(args.hold)


def stage(title, detail):
    event('stage', title=title, detail=detail)


def cli(*arguments, visible=True, ok=True):
    command = [str(BIN / 'log-print'), '--state', str(STATE), *map(str, arguments)]
    if visible:
        event('command', command=shlex.join(command))
    result = subprocess.run(command, capture_output=True, text=True, timeout=40, cwd=WORK)
    # Record every invocation, including bounded readiness polling. Polls are hidden only in the video UI.
    event('result', argv=command, stdout=result.stdout, stderr=result.stderr,
          code=result.returncode, visible=visible)
    if ok and result.returncode:
        raise AssertionError(f'CLI failed ({result.returncode}): {result.stderr}')
    if visible:
        pause()
    return result


def data(*arguments, visible=False):
    return json.loads(cli(*arguments, visible=visible).stdout)


def until(check, timeout=12):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = check()
        if value:
            return value
        time.sleep(.1)
    raise AssertionError('readiness condition exceeded deadline')


def assertion(name, passed, **evidence):
    checks.append(dict(name=name, passed=bool(passed), **evidence))
    event('assert', name=name, passed=bool(passed), evidence=evidence)
    if not passed:
        raise AssertionError(name)


def process_identity(pid):
    result = subprocess.run(['ps', '-ww', '-o', 'stat=', '-o', 'lstart=', '-o', 'command=', '-p', str(pid)],
                            capture_output=True, text=True, timeout=3)
    value = result.stdout.strip()
    if not value:
        return None
    state, identity = value.split(None, 1)
    return None if state.startswith('Z') else identity


def remember_processes():
    if STATE.exists():
        state = json.loads(STATE.read_text())
        for key in ['pid', 'core_pid']:
            owned[state[key]] = process_identity(state[key])
        snapshot = data('status')
        for plugin in snapshot['plugin_processes']:
            if plugin.get('pid'):
                owned[plugin['pid']] = process_identity(plugin['pid'])


def stream(owner):
    return next(s['id'] for s in data('streams') if s['owner'] == owner)


def records(identity):
    return data('read', identity)['records']


def bytes_of(rows):
    return b''.join(bytes(row['payload']) for row in rows)


def write_config(config):
    CONFIG.write_text(json.dumps(config, indent=2))
    # Only public static fixture config, never the runtime config containing generated tokens.
    (OUT / 'config.json').write_text(redact(CONFIG.read_text()) + '\n')


try:
    event('intro', title='文件采集 → 快照读取' if scenario == 'file-read' else '程序采集 → 转换 → 归档',
          detail='脚本驱动实际 CLI · 教学输入 · log-print 0.1.2')
    version = subprocess.run([str(BIN / 'log-print'), '--version'], capture_output=True, text=True, check=True).stdout.strip()
    event('environment', version=version, binaries={p.name: hashlib.sha256(p.read_bytes()).hexdigest()
          for p in BIN.iterdir() if p.name in ['log-print', 'log-print-core', 'input-file', 'input-program', 'output-transform', 'output-file']})
    assertion('version is 0.1.2', version == 'log-print 0.1.2')
    pause()
    if scenario == 'file-read':
        fixture = b'INFO demo=file step=boot\nWARN demo=file queue=3\nINFO demo=file step=done\n'
        (WORK / 'teaching.log').write_bytes(fixture)
        (OUT / 'teaching.log').write_bytes(fixture)
        write_config({'plugins':[{'id':'file','role':'input','bin':'input-file',
            'streams':[{'id':'demo-file','description':'Teaching fixture: three generated lines'}],
            'config':{'path':str(WORK/'teaching.log'),'mode':'static'}}]})
        stage('01 / 启动文件采集', 'input-file / static：读取教学文件；Core 保存有界内存快照。')
        cli('start', '--config', CONFIG)
        remember_processes()
        source = stream('file')
        until(lambda: bytes_of(records(source)) == fixture)
        stage('02 / 获取真实流 ID', 'streams 返回 Core 分配的 UUID；配置中的 demo-file 只是别名。')
        cli('streams')
        stage('03 / 读取原始字节', 'read --raw 返回当前保留的快照；这里没有创建持久化归档。')
        raw = cli('read', source, '--raw', '--wait-ms', '1000').stdout.encode()
        assertion('read bytes equal teaching file', raw == fixture, bytes=len(raw), sha256=hashlib.sha256(raw).hexdigest())
        report = next(p['report'] for p in data('status')['plugins'] if p['id']=='file')
        assertion('source EOF does not claim downstream completion', report['state']=='source_eof' and report['downstream_complete'] is False)
        stage('04 / 停止自建实例', 'stop 等待本次实例退出；读取一致性与进程退出均由脚本断言。')
        cli('stop')
    else:
        source_script = "import os,time\nos.write(1,b'INFO demo=program step=boot\\n')\ntime.sleep(0.2)\nos.write(2,b'WARN demo=program queue=3\\n')\ntime.sleep(0.2)\nos.write(1,b'INFO demo=program step=done\\n')\n"
        (WORK/'teaching.py').write_text(source_script)
        (OUT/'teaching.py').write_text(source_script)
        write_config({'plugins':[
            {'id':'source','role':'input','bin':'input-program','autostart':False,
             'streams':[{'id':'raw','description':'Teaching stdout and stderr'}],
             'config':{'command':sys.executable,'args':['-u',str(WORK/'teaching.py')]}},
            {'id':'transform','role':'output','bin':'output-transform','reads':['raw'],
             'streams':[{'id':'numbered','parents':['raw']}],
             'config':{'number':True,'timestamp':False,'reorder':True}},
            {'id':'archive','role':'output','bin':'output-file','reads':['numbered'],
             'config':{'mode':'create','file':{'format':'jsonl','paths':{'numbered':str(WORK/'numbered.jsonl')}},
                       'sqlite':{'path':str(WORK/'numbered.sqlite')},
                       'commit':{'max_records':1,'max_bytes':4096,'max_delay_ms':30}}}
        ]})
        stage('01 / 启动转换和归档', '预配置两个 Output；程序输入稍后启动。归档使用本次新建目录。')
        cli('start', '--config', CONFIG)
        remember_processes()
        original, derived = stream('source'), stream('transform')
        stage('02 / 启动教学程序', '真实 input-program 捕获 stdout / stderr；两个通道保留来源标签。')
        cli('plugin', 'start', 'source')
        remember_processes()
        until(lambda:len(records(derived)) == 3)
        source_report = next(p['report'] for p in data('status')['plugins'] if p['id']=='source')
        assertion('teaching program exited successfully', source_report['state']=='source_exited' and source_report['exit_code']==0)
        stage('03 / 查看原始流', '原始 payload 保持不变；跨 stdout / stderr 不承诺全局来源顺序。')
        cli('read', original, '--raw')
        raw_rows = records(original)
        assertion('both source channels captured', {r['channel'] for r in raw_rows} == {'stdout','stderr'})
        assertion('three original teaching lines preserved', sorted(bytes(r['payload']) for r in raw_rows) == sorted([
            b'INFO demo=program step=boot\n',b'WARN demo=program queue=3\n',b'INFO demo=program step=done\n']))
        stage('04 / 查看独立派生流', 'output-transform 在另一条流添加 [n=…]；原流没有被修改。')
        cli('read', derived, '--raw')
        derived_rows = records(derived)
        assertion('derived stream has a different UUID', original != derived)
        assertion('numbered output matches original payloads', [bytes(r['payload']) for r in derived_rows] ==
                  [f'[n={i+1}] '.encode()+bytes(r['payload']) for i,r in enumerate(raw_rows)])
        assertion('original stream unchanged', records(original) == raw_rows)
        stage('05 / 确认归档完成', '先停止转换，再停止归档；归档成功响应之后检查 JSONL 与 SQLite。')
        cli('plugin', 'stop', 'transform')
        until(lambda: data('plugin','call','archive','status.get').get('common',{}).get(derived,{}).get('next') == 4)
        stopped = data('plugin', 'stop', 'archive', visible=True)
        assertion('archive shutdown succeeded without force', stopped['success'] is True and stopped['forced'] is False)
        jsonl_rows = [json.loads(line)['record'] for line in (WORK/'numbered.jsonl').read_text().splitlines()]
        assertion('JSONL stores every derived record', jsonl_rows == derived_rows, records=len(jsonl_rows))
        with closing(sqlite3.connect(WORK/'numbered.sqlite')) as database:
            sql_rows = database.execute('SELECT payload,channel FROM records ORDER BY length(seq),seq').fetchall()
            integrity = database.execute('PRAGMA integrity_check').fetchone()[0]
        assertion('SQLite integrity check is ok', integrity == 'ok', result=integrity)
        assertion('SQLite matches the derived stream', sql_rows == [(bytes(r['payload']),r['channel']) for r in derived_rows], records=len(sql_rows))
        (OUT/'numbered.jsonl').write_bytes((WORK/'numbered.jsonl').read_bytes())
        event('verification', text='JSONL: 3 records / full metadata match\nSQLite: 3 records / payload + channel match\nPRAGMA integrity_check: ok\nOriginal stream: unchanged')
        pause()
        stage('06 / 停止自建实例', '只清理本次 state 对应的实例；Core、插件与源程序均已结束。')
        cli('stop')
    until(lambda: all(identity is None or process_identity(pid) != identity for pid,identity in owned.items()))
    assertion('state removed after stop', not STATE.exists())
    assertion('all observed owned processes exited', all(identity is None or process_identity(pid) != identity for pid,identity in owned.items()), pids=list(owned))
    event('complete', title='验证通过 · 自建实例已停止', text=f'{len(checks)} 项断言通过。演示 payload 均为教学输入。')
    completed = True
    pause()
finally:
    # Never signal by a broad name or touch other instances. Stop through the private state only.
    if STATE.exists():
        cli('stop', visible=False, ok=False)
    result = {'scenario':scenario,'checks':checks,'passed':completed and bool(checks) and all(x['passed'] for x in checks),
              'state_removed':not STATE.exists(),'processes_exited':all(identity is None or process_identity(pid) != identity for pid,identity in owned.items()),
              'redactions':['binary directory -> <bin-dir>','temporary fixture directory -> <demo-dir>','repository -> <repo>',
                            'Python executable -> <python3>'],
              'recorded_at':time.strftime('%Y-%m-%dT%H:%M:%S%z')}
    (OUT/'assertions.json').write_text(json.dumps(result, ensure_ascii=False, indent=2)+'\n')
    temporary.cleanup()
