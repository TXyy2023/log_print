import fs from 'node:fs';
import path from 'node:path';
import { spawn, spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
const home=path.dirname(fileURLToPath(import.meta.url)), root=path.dirname(path.dirname(home));
process.chdir(home);
function prepare(refresh=false) {
 const result=spawnSync(process.execPath,['prepare.mjs','local',...(refresh?['--refresh']:[])],{stdio:'inherit'});
 return result.status===0;
}
if(!prepare()) process.exit(1);
const vp=spawn(process.execPath,['node_modules/vitepress/bin/vitepress.js','dev','.cache/local','--host','127.0.0.1','--port','5173','--strictPort'],{stdio:'inherit'});
let timer;
function update(){clearTimeout(timer);timer=setTimeout(()=>prepare(true),220);}
const watchers=['public','local'].map(section=>fs.watch(path.join(root,'doc',section),{recursive:true},update));
watchers.push(fs.watch(path.join(root,'doc/README.md'),update));
const inputs=JSON.parse(fs.readFileSync('.cache/local/build-inputs.json'));
for(const rel of inputs.filter(p=>!p.startsWith('doc/'))) {const p=path.join(root,rel);if(fs.existsSync(p))watchers.push(fs.watch(p,update));}
for(const name of ['prepare.mjs','theme.js','Mermaid.vue','style.css','public-pages.json','public-navigation.json','public-sources.json','archive-origins.json','legacy-paths.json','repository-paths.json','repository-origins.json']) watchers.push(fs.watch(name,update));
let ending=false;
function stop(){if(ending)return;ending=true;clearTimeout(timer);for(const w of watchers)w.close();vp.kill('SIGTERM');}
process.on('SIGTERM',stop);process.on('SIGINT',stop);
vp.on('exit',code=>{stop();process.exit(code||0);});
