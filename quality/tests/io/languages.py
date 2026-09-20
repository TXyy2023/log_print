#!/usr/bin/env python3
"""Exercise installed language tools through input-program -> Core -> output-raw.

Python and Rust are required. Missing optional tools are explicit skips. An
installed tool that cannot build/run the fixture is a failure, not a support
claim. All sources, build products, Go caches and runtimes are local temporary
fixtures. This script does not install language runtimes or alter global config.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tempfile
import time

import verify

OUT = b'OUT\x00\xffno-newline:part1'
ERR = b'ERR\x00\xffno-newline:part1'
TAIL = b':part2'


def first_tool(names):
    return next((path for name in names if (path := shutil.which(name))), None)


def checked(command, directory, env=None):
    result = subprocess.run(command, cwd=directory, env=env, capture_output=True,
                            text=True, errors='replace', timeout=120)
    if result.returncode:
        raise RuntimeError(f'fixture command failed ({result.returncode}): {command[0]}\n'
                           + result.stdout[-3000:] + result.stderr[-3000:])
    return result.stdout.strip() or result.stderr.strip()


def numbers(payload):
    return ','.join(map(str, payload))


def prepare(language, tool, directory):
    gate = directory/'continue'
    version = checked([tool, '--version'], directory).splitlines()[0] if language in ['python','rust','node','c','cpp'] and Path(tool).name.lower() not in ['cl','cl.exe'] else None
    if language == 'python':
        path = directory/'source.py'
        path.write_text(f'''import os,sys,time
os.write(1,bytes([{numbers(OUT)}]));os.write(2,bytes([{numbers(ERR)}]))
end=time.monotonic()+15
while not os.path.exists(sys.argv[1]):
    if time.monotonic()>end:raise RuntimeError("gate timeout")
    time.sleep(0.01)
os.write(1,b":part2");os.write(2,b":part2")
''')
        return [tool,'-u',str(path),str(gate)], version
    if language == 'node':
        path = directory/'source.js'
        path.write_text(f'''const fs=require('node:fs');
fs.writeSync(1,Buffer.from([{numbers(OUT)}]));fs.writeSync(2,Buffer.from([{numbers(ERR)}]));
const start=Date.now();
const timer=setInterval(()=>{{
  if(fs.existsSync(process.argv[2])){{clearInterval(timer);fs.writeSync(1,Buffer.from(':part2'));fs.writeSync(2,Buffer.from(':part2'));}}
  else if(Date.now()-start>15000){{clearInterval(timer);throw Error('gate timeout');}}
}},10);
''')
        return [tool,str(path),str(gate)], version
    if language == 'rust':
        path=directory/'source.rs';binary=directory/('source'+verify.SUFFIX)
        path.write_text(f'''use std::io::Write;
fn main() {{
 let mut out=std::io::stdout();let mut err=std::io::stderr();
 out.write_all(&[{numbers(OUT)}]).unwrap();out.flush().unwrap();
 err.write_all(&[{numbers(ERR)}]).unwrap();err.flush().unwrap();
 let gate=std::env::args().nth(1).unwrap();let start=std::time::Instant::now();
 while !std::path::Path::new(&gate).exists(){{assert!(start.elapsed().as_secs()<15,"gate timeout");std::thread::sleep(std::time::Duration::from_millis(10));}}
 out.write_all(b":part2").unwrap();out.flush().unwrap();err.write_all(b":part2").unwrap();err.flush().unwrap();
}}
''')
        checked([tool,str(path),'-o',str(binary)],directory)
        return [str(binary),str(gate)],version
    if language in ['c','cpp']:
        cpp=language=='cpp';path=directory/('source.cpp'if cpp else'source.c');binary=directory/('source'+verify.SUFFIX)
        shared=f'''#include <stdio.h>
#ifdef _WIN32
#include <io.h>
#include <fcntl.h>
#include <windows.h>
#define PAUSE() Sleep(10)
#else
#include <unistd.h>
#define PAUSE() usleep(10000)
#endif
static const unsigned char out_data[]={{{numbers(OUT)}}};
static const unsigned char err_data[]={{{numbers(ERR)}}};
'''
        if cpp:
            shared+='''#include <iostream>
int main(int argc,char**argv){
#ifdef _WIN32
_setmode(_fileno(stdout),_O_BINARY);_setmode(_fileno(stderr),_O_BINARY);
#endif
if(argc!=2)return 2;
std::cout.write(reinterpret_cast<const char*>(out_data),sizeof(out_data));std::cout.flush();
std::cerr.write(reinterpret_cast<const char*>(err_data),sizeof(err_data));std::cerr.flush();
for(int n=0;n<1500;n++){FILE*f=fopen(argv[1],"rb");if(f){fclose(f);std::cout.write(":part2",6);std::cout.flush();std::cerr.write(":part2",6);std::cerr.flush();return 0;}PAUSE();}
return 3;}
'''
        else:
            shared+='''int main(int argc,char**argv){
#ifdef _WIN32
_setmode(_fileno(stdout),_O_BINARY);_setmode(_fileno(stderr),_O_BINARY);
#endif
if(argc!=2)return 2;
if(fwrite(out_data,1,sizeof(out_data),stdout)!=sizeof(out_data)||fflush(stdout))return 4;
if(fwrite(err_data,1,sizeof(err_data),stderr)!=sizeof(err_data)||fflush(stderr))return 4;
for(int n=0;n<1500;n++){FILE*f=fopen(argv[1],"rb");if(f){fclose(f);if(fwrite(":part2",1,6,stdout)!=6||fflush(stdout))return 4;if(fwrite(":part2",1,6,stderr)!=6||fflush(stderr))return 4;return 0;}PAUSE();}
return 3;}
'''
        path.write_text(shared)
        if Path(tool).name.lower()in ['cl','cl.exe']:
            details=subprocess.run([tool],capture_output=True,text=True,errors='replace',timeout=10)
            version=(details.stderr or details.stdout).splitlines()[0]
            command=[tool,'/nologo',str(path),'/Fe:'+str(binary)]
            if cpp:command+=['/EHsc','/std:c++17']
        else:
            command=[tool,str(path),'-o',str(binary)]
            if cpp:command+=['-std=c++17']
        checked(command,directory)
        return [str(binary),str(gate)],version
    if language=='go':
        path=directory/'source.go';binary=directory/('source'+verify.SUFFIX)
        path.write_text(f'''package main
import("os";"time")
func main(){{
 if _,e:=os.Stdout.Write([]byte{{{numbers(OUT)}}});e!=nil{{panic(e)}}
 if _,e:=os.Stderr.Write([]byte{{{numbers(ERR)}}});e!=nil{{panic(e)}}
 start:=time.Now()
 for {{if _,e:=os.Stat(os.Args[1]);e==nil{{break}};if time.Since(start)>15*time.Second{{panic("gate timeout")}};time.Sleep(10*time.Millisecond)}}
 if _,e:=os.Stdout.Write([]byte(":part2"));e!=nil{{panic(e)}}
 if _,e:=os.Stderr.Write([]byte(":part2"));e!=nil{{panic(e)}}
}}
''')
        for name in ['gocache','gopath','gotmp','config']:(directory/name).mkdir()
        env=dict(os.environ,GOCACHE=str(directory/'gocache'),GOPATH=str(directory/'gopath'),
                 GOTMPDIR=str(directory/'gotmp'),GOENV='off',GOTOOLCHAIN='local',
                 GOPROXY='off',GOSUMDB='off',GO111MODULE='off',GOFLAGS='',
                 XDG_CONFIG_HOME=str(directory/'config'),GOTELEMETRY='off')
        version=checked([tool,'version'],directory,env).splitlines()[0]
        checked([tool,'build','-o',str(binary),str(path)],directory,env)
        return [str(binary),str(gate)],version
    if language=='shell' and os.name!='nt':
        path=directory/'source.sh'
        def octal(payload):return ''.join(f'\\{b:03o}' for b in payload)
        path.write_text(f'''set -eu
printf '{octal(OUT)}'
printf '{octal(ERR)}' >&2
n=0
while [ ! -f "$1" ]; do
 n=$((n+1)); [ "$n" -lt 1500 ] || exit 3
 sleep 0.01
done
printf ':part2'
printf ':part2' >&2
''')
        version=checked([tool,'-c','printf "%s\\n" "${BASH_VERSION:-POSIX shell; version unavailable}"'],directory)
        return [tool,str(path),str(gate)],version
    if language=='shell':
        path=directory/'source.ps1'
        path.write_text(f'''param([string]$Gate)
$ErrorActionPreference='Stop'
$out=[Console]::OpenStandardOutput();$err=[Console]::OpenStandardError()
[byte[]]$a=@({numbers(OUT)});[byte[]]$b=@({numbers(ERR)})
$out.Write($a,0,$a.Length);$out.Flush();$err.Write($b,0,$b.Length);$err.Flush()
$until=[DateTime]::UtcNow.AddSeconds(15)
while(-not(Test-Path -LiteralPath $Gate)){{if([DateTime]::UtcNow -gt $until){{throw 'gate timeout'}};Start-Sleep -Milliseconds 10}}
[byte[]]$tail=@({numbers(TAIL)});$out.Write($tail,0,$tail.Length);$out.Flush();$err.Write($tail,0,$tail.Length);$err.Flush()
''')
        version=checked([tool,'-NoLogo','-NoProfile','-NonInteractive','-Command','$PSVersionTable.PSVersion.ToString()'],directory)
        return [tool,'-NoLogo','-NoProfile','-NonInteractive','-File',str(path),'-Gate',str(gate)],version
    raise ValueError(language)


def exercise(directory,command):
    out,err=directory/'stdout.raw',directory/'stderr.raw'
    specs=[verify.plugin('source','input-program',{'command':command[0],'args':command[1:],
        'stdout_stream':'stdout','stderr_stream':'stderr','chunk_bytes':3},['stdout','stderr']),
        verify.plugin('raw','output-raw',{'streams':['stdout','stderr'],
        'paths':{'stdout':str(out),'stderr':str(err)}},reads=['stdout','stderr'])]
    runtime=verify.Runtime(directory,specs)
    try:
        runtime.start('raw');process=runtime.start('source')
        def first_bytes():
            if process.poll()is not None:
                raise AssertionError('source exited before no-newline observation: '+(directory/'source.stderr').read_text(errors='replace'))
            return verify.content(out)==OUT and verify.content(err)==ERR
        verify.wait(first_bytes,'first bytes were not pushed while source remained running',seconds=10)
        (directory/'continue').write_bytes(b'continue')
        assert process.wait(timeout=10)==0,(directory/'source.stderr').read_text(errors='replace')
        verify.wait(lambda:verify.content(out)==OUT+TAIL and verify.content(err)==ERR+TAIL,'final stdout/stderr bytes differ')
        runtime.stop('raw')
        assert verify.content(out)==OUT+TAIL and verify.content(err)==ERR+TAIL
        return {'stdout_bytes':len(OUT+TAIL),'stderr_bytes':len(ERR+TAIL),
                'stdout_sha256':hashlib.sha256(OUT+TAIL).hexdigest(),
                'stderr_sha256':hashlib.sha256(ERR+TAIL).hexdigest(),
                'nul_and_non_utf8_preserved':True,'observed_before_source_exit':True,
                'source_chunk_bytes':3}
    finally:runtime.close()


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bin-dir',type=Path,default=verify.BIN)
    parser.add_argument('--json',type=Path,help='optional output artifact; stdout always receives JSON')
    args=parser.parse_args();verify.BIN=args.bin_dir.resolve()
    tools={'python':sys.executable,'rust':first_tool(['rustc']), 'node':first_tool(['node']),
           'c':first_tool(['cc','clang','gcc','cl']), 'cpp':first_tool(['c++','clang++','g++','cl']),
           'go':first_tool(['go']), 'shell':first_tool(['pwsh','powershell'])if os.name=='nt'else first_tool(['sh','bash'])}
    result={'platform':platform.platform(),'machine':platform.machine(),
            'executed_at_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),
            'transport':'actual language program -> input-program -> Core -> output-raw',
            'binaries':{name:hashlib.sha256((verify.BIN/(name+verify.SUFFIX)).read_bytes()).hexdigest()
                        for name in ['input-program','log-print-core','output-raw']},'cases':[]}
    for language,tool in tools.items():
        item={'language':language,'tool':tool,'required':language in ['python','rust']}
        if not tool:
            item.update(status='failed'if item['required']else'skipped',reason='tool not found on PATH')
        else:
            with tempfile.TemporaryDirectory(prefix=f'log-print-language-{language}-')as temporary:
                directory=Path(temporary)
                try:
                    command,version=prepare(language,tool,directory);item['version']=version
                    item.update(exercise(directory,command));item['status']='passed'
                except Exception as error:item.update(status='failed',reason=str(error))
        result['cases'].append(item)
        print(f"{language}: {item['status']}",file=sys.stderr,flush=True)
    result['passed']=sum(c['status']=='passed'for c in result['cases'])
    result['skipped']=sum(c['status']=='skipped'for c in result['cases'])
    result['failed']=sum(c['status']=='failed'for c in result['cases'])
    encoded=json.dumps(result,ensure_ascii=False,indent=2)
    if args.json:args.json.parent.mkdir(parents=True,exist_ok=True);args.json.write_text(encoded+'\n')
    print(encoded)
    return 1 if result['failed']else 0


if __name__=='__main__':raise SystemExit(main())
