// Real Chromium acceptance against an embedded Rust server. No app mocks.
import { chromium, expect } from '../../../project/plugins/outputs/output-webui/frontend/node_modules/@playwright/test/index.mjs';
import { execFileSync } from 'node:child_process';
import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
const [url, state, binary, artifact] = process.argv.slice(2);
await mkdir(artifact,{recursive:true});
const browser = await chromium.launch();
const context = await browser.newContext({viewport:{width:1600,height:1050}});
await context.tracing.start({screenshots:true,snapshots:true});
const first=await context.newPage(), second=await context.newPage();
const errors=[];
for(const p of [first,second]){p.on('pageerror',e=>errors.push(String(e)));p.on('console',m=>{if(m.type()==='error')errors.push(m.text());});}
const cli=(...args)=>execFileSync(binary,['--state',state,'webui','web',...args],{encoding:'utf8'});
const read=async()=>await (await context.request.get(url+'/api/state')).json();
const control=async(method,args={})=>{const r=await context.request.post(url+'/api/control',{headers:{Origin:url},data:{method,args}});expect(r.ok()).toBeTruthy();return await r.json();};
let ok=false;
try{
 await first.goto(url);await second.goto(url);
 cli('page','set','--title','Browser acceptance','--sidebar-open','false','--view-x','24','--view-y','24','--view-zoom','1');
 cli('panel','add','--title','Console CI','--left','0','--top','0','--panel-width','760','--panel-height','440','--column','text');
 let config=await read();const log=config.pages[0].panels[0].id;
 cli('panel','add','--kind','curve','--title','Temperature CI','--left','800','--top','0','--panel-width','600','--panel-height','440');
 config=await read();const curve=config.pages[0].panels[1].id;
 cli('series','add','--panel',curve,'--name','Temperature','--regex','temperature=(?P<value>[0-9.]+)');
 const node=p=>p.locator(`.vue-flow__node[data-id="${log}"]`);
 await expect(node(first).locator('.ag-row').first()).toBeVisible();
 await expect(node(second).locator('.panel-drag')).toContainText('Console CI');
 expect(await node(first).locator('.ag-row').count()).toBeLessThan(100);
 await expect(first.locator('.chart canvas')).toBeVisible();
 // Persisted dragging and resizing of the real VueFlow node.
 cli('page','set','--active-panel',log);
 const drag=await node(first).locator('.panel-drag').boundingBox();
 await first.mouse.move(drag.x+80,drag.y+12);await first.mouse.down();await first.mouse.move(drag.x+128,drag.y+36,{steps:12});await first.mouse.up();
 await expect.poll(async()=>(await read()).pages[0].panels[0].left).toBeGreaterThan(0);
 const resize=node(first).locator('.vue-flow__resize-control.handle.bottom.right');
 await expect(resize).toBeVisible();const rb=await resize.boundingBox();
 await first.mouse.move(rb.x+rb.width/2,rb.y+rb.height/2);await first.mouse.down();await first.mouse.move(rb.x+rb.width/2+64,rb.y+rb.height/2+48,{steps:12});await first.mouse.up();
 await expect.poll(async()=>(await read()).pages[0].panels[0].panel_width).toBeGreaterThan(760);
 // Filter + CLI synchronization + fixed history view from actual archive.
 await node(first).getByRole('textbox',{name:'筛选日志',exact:true}).fill('temperature=1');
 await node(first).getByRole('textbox',{name:'筛选日志',exact:true}).press('Enter');
 await expect(node(second).getByRole('textbox',{name:'筛选日志',exact:true})).toHaveValue('temperature=1');
 await node(first).getByRole('button',{name:'历史',exact:true}).click();
 await expect.poll(async()=>(await read()).pages[0].panels[0].mode).toBe('history');
 await expect(node(first).locator('.ag-row').first()).toBeVisible();
 cli('panel','set','--panel',log,'--mode','live','--text','');
 await expect(node(second).getByRole('textbox',{name:'筛选日志',exact:true})).toHaveValue('');
 // Real ECharts wheel zoom updates the backend and therefore the second client.
 const chart=first.locator('.chart canvas');const cb=await chart.boundingBox();
 await first.mouse.move(cb.x+cb.width/2,cb.y+cb.height/2);await first.mouse.wheel(0,-300);
 await expect.poll(async()=>(await read()).pages[0].panels[1].zoom_start??0).toBeGreaterThan(0);
 const saved=(await read()).pages[0];await first.reload();
 await expect(node(first)).toBeVisible();expect((await read()).pages[0]).toEqual(saved);
 // GridStack compatibility is an interactive renderer, not just stored JSON.
 cli('page','set','--layout-mode','grid');
 cli('panel','set','--panel',log,'--x','0','--y','0','--w','6','--h','6');
 cli('panel','set','--panel',curve,'--x','6','--y','0','--w','6','--h','6');
 const grid=first.locator(`.grid-stack-item[gs-id="${log}"]`);await expect(grid).toBeVisible();
 await grid.hover();
 const handle=await grid.locator('.ui-resizable-se').boundingBox();
 await first.mouse.move(handle.x+handle.width/2,handle.y+handle.height/2);await first.mouse.down();await first.mouse.move(handle.x+handle.width/2-120,handle.y+handle.height/2+72,{steps:12});await first.mouse.up();
 await expect.poll(async()=>(await read()).pages[0].panels[0].h).toBeGreaterThan(6);
 await first.screenshot({path:path.join(artifact,'gridstack.png'),fullPage:true});
 cli('page','set','--layout-mode','canvas');await expect(node(first)).toBeVisible();
 await first.screenshot({path:path.join(artifact,'workbench.png'),fullPage:true});
 expect(errors).toEqual([]);ok=true;
}finally{
 await writeFile(path.join(artifact,'browser.json'),JSON.stringify({ok,errors,platform:process.platform},null,2));
 if(!ok)await first.screenshot({path:path.join(artifact,'failure.png'),fullPage:true}).catch(()=>{});
 await context.tracing.stop({path:path.join(artifact,'trace.zip')});await browser.close();
}
