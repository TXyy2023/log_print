// Real CLI event viewer + browser recording. See README.md for dependencies and provenance.
import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { parseArgs } from 'node:util';

const { values } = parseArgs({ options: {
  scenario: { type: 'string' }, 'bin-dir': { type: 'string' },
  'playwright-module': { type: 'string' }, 'hold': { type: 'string', default: '3.3' },
} });
if (!['file-read', 'program-archive'].includes(values.scenario) || !values['bin-dir']) {
  throw new Error('Required: --scenario file-read|program-archive --bin-dir /absolute/binaries');
}
const { chromium } = await import(values['playwright-module'] ? pathToFileURL(path.resolve(values['playwright-module'])).href : 'playwright');
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const scenario = values.scenario;
const output = path.join(root, 'quality/artifacts/showcase-demo-recordings', scenario);
const publicDir = path.join(root, 'doc/public/assets/demos');
const basename = scenario === 'file-read' ? '01-file-read' : '02-transform-archive';
await mkdir(output, { recursive: true });
await mkdir(publicDir, { recursive: true });
const state = {
  title: scenario === 'file-read' ? '从文件，到可读的日志流' : '从程序输出，到持久化归档',
  subtitle: scenario === 'file-read' ? 'File input → Core stream → CLI snapshot' : 'Program → Transform → JSONL + SQLite',
  stage: '准备演示', detail: '真实 CLI · 教学输入 · 独立实例', command: '', output: '',
  checks: [], phase: '准备', step: 0, elapsed: 0, outputNote: '原始 stdout / stderr（仅路径替换）',
};
const html = `<!doctype html><html lang="zh-CN"><meta charset="utf-8"><title>log-print actual CLI demo</title><style>
*{box-sizing:border-box}body{margin:0;width:1600px;height:900px;background:#080d19;color:#e9f1fc;font-family:-apple-system,BlinkMacSystemFont,'PingFang SC',sans-serif;padding:35px 44px;overflow:hidden}header{display:flex;align-items:center;justify-content:space-between;padding-bottom:25px;border-bottom:1px solid #263145}.brand{display:flex;align-items:center;gap:16px;font-size:29px;font-weight:750;letter-spacing:-.8px}.icon{display:flex;width:42px;height:42px;align-items:center;justify-content:center;border:1px solid #74e0be;border-radius:12px;background:#10372f;color:#9bf5d6;font:700 24px ui-monospace,monospace}.pills{display:flex;gap:10px}.pill{border:1px solid #2d3b52;border-radius:30px;padding:8px 15px;font:15px ui-monospace,monospace;color:#b5c8df}h1{font-size:40px;letter-spacing:-1.2px;margin:28px 0 6px}.subtitle{font:17px ui-monospace,monospace;color:#8a9db9;margin-bottom:24px}.main{display:grid;grid-template-columns:350px 1fr;gap:26px;height:563px}.info{display:flex;flex-direction:column;padding:24px;border:1px solid #233048;background:linear-gradient(145deg,#101c2e,#0b1321);border-radius:16px}.eyebrow{color:#66d8b3;font-size:15px;letter-spacing:1px;margin-bottom:18px}h2{font-size:26px;line-height:1.4;margin:0 0 14px;min-height:73px}.detail{color:#b2c3da;font-size:19px;line-height:1.65;min-height:110px}.validation{margin-top:auto;padding-top:22px;border-top:1px solid #29394f}.validation-title{color:#6e8ba8;font:13px ui-monospace,monospace;letter-spacing:.8px;margin-bottom:13px}.check{font-size:14px;line-height:1.6;color:#93dbc2;margin-top:8px;overflow-wrap:anywhere}.terminal{min-width:0;border:1px solid #2b3b56;border-radius:16px;background:#0c1422;overflow:hidden;display:flex;flex-direction:column}.bar{display:flex;justify-content:space-between;background:#131e31;padding:14px 20px;font:14px ui-monospace,monospace;color:#8ba1bf}.dots{color:#58738d;letter-spacing:5px}.command{margin:0;padding:18px 23px;background:#0e1b2c;border-bottom:1px solid #24364c;color:#9abbff;white-space:pre-wrap;overflow-wrap:anywhere;font:18px/1.55 ui-monospace,SFMono-Regular,Menlo,monospace;min-height:66px}pre.output{margin:0;padding:17px 23px;white-space:pre-wrap;overflow-wrap:anywhere;font:18px/1.42 ui-monospace,SFMono-Regular,Menlo,monospace;color:#d0e2ee;overflow:hidden;flex:1}.strip{display:flex;justify-content:space-between;color:#7289a5;padding:9px 23px;border-top:1px solid #1d2b40;font-size:12px}.bottom{margin-top:24px;display:flex;justify-content:space-between;align-items:center}.disclosure{font-size:17px;color:#7890aa}.live{font-size:15px;color:#a0cbbd}.dot{display:inline-block;width:7px;height:7px;border-radius:50%;background:#68dab3;margin-right:8px}
</style><header><div class="brand"><span class="icon">&gt;_</span>log-print</div><div class="pills"><span class="pill">Rust</span><span class="pill">v0.1.2</span><span class="pill">TCP / localhost</span></div></header><h1 id="title"></h1><div class="subtitle" id="subtitle"></div><div class="main"><aside class="info"><div class="eyebrow">实际 CLI 工作流</div><h2 id="stage"></h2><div class="detail" id="detail"></div><div class="validation"><div class="validation-title">LIVE ASSERTIONS</div><div id="checks"></div></div></aside><section class="terminal"><div class="bar"><span><span class="dots">● ● ●</span> terminal output</span><span id="phase"></span></div><pre class="command" id="command"></pre><pre class="output" id="output"></pre><div class="strip"><span id="overflow"></span><span id="elapsed"></span></div></section></div><div class="bottom"><div class="disclosure">脚本驱动实际 CLI · 教学输入 · 路径占位脱敏 · 完整命令输出随源码提供</div><div class="live"><span class="dot"></span>实时事件视图录像</div></div><script>
async function update(){const s=await(await fetch('/state')).json();for(const k of ['title','subtitle','stage','detail','phase'])document.getElementById(k).textContent=s[k];document.getElementById('command').textContent=s.command?'$ '+s.command:'$ log-print --version';const lines=s.output.trimEnd().split('\\n');const output=document.getElementById('output');let shown=lines.slice(-17);output.textContent=shown.join('\\n');while(output.scrollHeight>output.clientHeight+1&&shown.length>1){shown.shift();output.textContent=shown.join('\\n')}document.getElementById('overflow').textContent=lines.length>shown.length?'原始输出末 '+shown.length+' 行 / 共 '+lines.length+' 行':s.outputNote;document.getElementById('elapsed').textContent=s.elapsed+'s';const box=document.getElementById('checks');box.replaceChildren();for(const c of s.checks.slice(-4)){const el=document.createElement('div');el.className='check';el.textContent=(c.passed?'✓ ':'✗ ')+c.name;box.append(el)}}setInterval(update,120);update();</script></html>`;
const server = createServer((req, res) => {
  res.setHeader('Cache-Control','no-store');
  res.setHeader('Content-Type',req.url==='/state'?'application/json':'text/html; charset=utf-8');
  res.end(req.url==='/state'?JSON.stringify(state):html);
});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
const browser = await chromium.launch({headless:true});
const context = await browser.newContext({viewport:{width:1600,height:900},recordVideo:{dir:output,size:{width:1600,height:900}},deviceScaleFactor:1});
const page = await context.newPage();
const errors=[];
page.on('pageerror', e=>errors.push(e.message));
await page.goto(`http://127.0.0.1:${server.address().port}`);
await page.waitForTimeout(1300);
const started=Date.now();
const timer=setInterval(()=>state.elapsed=Math.round((Date.now()-started)/1000),500);
const child=spawn('python3',[path.join(root,'quality/demos/run_demo.py'),scenario,'--bin-dir',values['bin-dir'],'--output',output,'--hold',values.hold],{cwd:root,stdio:['ignore','pipe','pipe']});
let pending='',stderr='',coverPromise=Promise.resolve(),coverSaved=false;
child.stderr.on('data', data=>stderr+=data.toString());
child.stdout.on('data', data=>{
  pending+=data.toString();let newline;
  while((newline=pending.indexOf('\n'))>=0){
    const line=pending.slice(0,newline);pending=pending.slice(newline+1);
    const e=JSON.parse(line);
    if(e.kind==='stage'){state.stage=e.title;state.detail=e.detail;state.phase='运行中';}
    if(e.kind==='environment')state.output=e.version;
    if(e.kind==='command'){state.command=e.command.replace('<bin-dir>/log-print','log-print');state.output='';}
    if(e.kind==='result'&&e.visible){
      state.output=e.stdout+(e.stderr?'\n[stderr]\n'+e.stderr:'');state.phase='exit '+e.code;state.outputNote='原始 stdout / stderr（仅路径替换）';
      const isCover=scenario==='file-read'?state.stage.startsWith('03 /'):state.stage.startsWith('04 /');
      if(isCover&&!coverSaved){coverSaved=true;coverPromise=page.waitForTimeout(450).then(()=>page.screenshot({path:path.join(publicDir,basename+'.png')}));}
    }
    if(e.kind==='assert')state.checks.push(e);
    if(e.kind==='verification'){state.output=e.text;state.command='[Python assertions against JSONL + SQLite]';state.phase='校验结果';state.outputNote='Python 校验结果 · 检查实际归档文件';}
    if(e.kind==='complete'){state.stage=e.title;state.detail=e.text;state.phase='完成';}
    if(['stage','complete'].includes(e.kind))process.stdout.write(line+'\n');
  }
});
const code=await new Promise((resolve,reject)=>{child.once('error',reject);child.once('close',resolve);});
clearInterval(timer);
await coverPromise;
if(code!==0){state.phase='失败';state.output+=stderr;}
await page.waitForTimeout(1400);
await page.screenshot({path:path.join(output,'final.png')});
const viewport=await page.evaluate(()=>({overflow:document.documentElement.scrollWidth>innerWidth||document.documentElement.scrollHeight>innerHeight}));
const video=page.video();
await context.close();
await video.saveAs(path.join(output,'live.webm'));
await browser.close();
await new Promise(resolve=>server.close(resolve));
await writeFile(path.join(output,'stderr.log'),stderr);
await writeFile(path.join(output,'recording.json'),JSON.stringify({scenario,code,elapsedSeconds:(Date.now()-started)/1000,width:1600,height:900,errors,viewport,coverSaved,source:'Script-driven real CLI; teaching fixtures',redaction:'Only known binary, repository and temporary fixture directory prefixes, and the Python executable'},null,2)+'\n');
if(code!==0||errors.length||viewport.overflow||!coverSaved)throw new Error('Recording failed: '+stderr+JSON.stringify({code,errors,viewport,coverSaved}));
const ffmpegArgs=['-y','-i',path.join(output,'live.webm'),'-an','-c:v','libx264','-preset','medium','-crf','23','-pix_fmt','yuv420p','-movflags','+faststart','-r','25',path.join(publicDir,basename+'.mp4')];
const encode=spawn('ffmpeg',ffmpegArgs,{stdio:['ignore','ignore','pipe']});let encodeLog='';encode.stderr.on('data',data=>encodeLog+=data.toString());
const encodeCode=await new Promise((resolve,reject)=>{encode.once('error',reject);encode.once('close',resolve);});
await writeFile(path.join(output,'ffmpeg.log'),encodeLog);
if(encodeCode!==0)throw new Error('ffmpeg failed: '+encodeLog);
const probe=spawn('ffprobe',['-v','error','-show_entries','stream=codec_name,width,height,pix_fmt,r_frame_rate:format=duration,size','-of','json',path.join(publicDir,basename+'.mp4')],{stdio:['ignore','pipe','pipe']});let metadata='';probe.stdout.on('data',data=>metadata+=data.toString());
const probeCode=await new Promise((resolve,reject)=>{probe.once('error',reject);probe.once('close',resolve);});
const parsed=JSON.parse(metadata);
if(probeCode!==0||parsed.streams[0].codec_name!=='h264'||Number(parsed.format.size)>10_000_000)throw new Error('MP4 verification failed');
await writeFile(path.join(output,'ffprobe.json'),metadata);
// Publish redacted evidence next to the reproducible driver, excluding private runtime files.
const evidence=path.join(root,'quality/demos/evidence',scenario);
await mkdir(evidence,{recursive:true});
for(const filename of ['transcript.jsonl','assertions.json','config.json','recording.json','ffprobe.json',scenario==='file-read'?'teaching.log':'teaching.py',...(scenario==='program-archive'?['numbered.jsonl']:[])]){
  await writeFile(path.join(evidence,filename),await readFile(path.join(output,filename)));
}
console.log(JSON.stringify({complete:true,scenario,video:path.join(publicDir,basename+'.mp4'),...parsed.format}));
