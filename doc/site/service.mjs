import fs from 'node:fs';
import path from 'node:path';
import net from 'node:net';
import {spawn,execFileSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
const home=path.dirname(fileURLToPath(import.meta.url));
const run=path.join(home,'.cache');fs.mkdirSync(run,{recursive:true});
const state=path.join(run,'service.json'),log=path.join(run,'service.log');
const current=fs.existsSync(state)?JSON.parse(fs.readFileSync(state)):null;
function alive(){if(!current)return false;try{process.kill(current.pid,0);return execFileSync('ps',['-p',String(current.pid),'-o','command='],{encoding:'utf8'}).includes(path.join(home,'dev.mjs'));}catch{return false;}}
const command=process.argv[2];
if(command==='status'){console.log(alive()?`Running PID ${current.pid}: http://127.0.0.1:5173/`:'Stopped');}
else if(command==='stop'){if(alive())process.kill(current.pid,'SIGTERM');if(fs.existsSync(state))fs.unlinkSync(state);console.log('Stop requested for this documentation service.');}
else if(command==='start'){
 if(alive()){console.log(`Already running: http://127.0.0.1:5173/`);process.exit(0);}
 await new Promise((resolve,reject)=>{const server=net.createServer();server.once('error',reject);server.listen(5173,'127.0.0.1',()=>server.close(resolve));});
 const fd=fs.openSync(log,'a');const child=spawn(process.execPath,[path.join(home,'dev.mjs')],{cwd:home,detached:true,stdio:['ignore',fd,fd]});
 child.unref();fs.closeSync(fd);fs.writeFileSync(state,JSON.stringify({pid:child.pid,started:new Date().toISOString(),url:'http://127.0.0.1:5173/'}));
 console.log(`Started PID ${child.pid}; log: ${log}`);
}else throw Error('Use start, stop or status');
