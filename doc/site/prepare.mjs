import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
const home = path.dirname(fileURLToPath(import.meta.url));
const doc = path.dirname(home), root = path.dirname(doc);
const mode = process.argv[2];
if (!['local', 'public'].includes(mode)) throw Error('Use local or public');
const out = path.join(home, '.cache', mode);
const refresh = process.argv.includes('--refresh');
if(!refresh) fs.rmSync(out, { recursive: true, force: true });
fs.mkdirSync(out, { recursive: true });
const generated = [];
const previous = refresh && fs.existsSync(path.join(out,'.generated.json')) ? JSON.parse(fs.readFileSync(path.join(out,'.generated.json'))) : [];
const legacy = mode === 'local' ? JSON.parse(fs.readFileSync(path.join(home, 'legacy-paths.json'))) : {};
const allow = JSON.parse(fs.readFileSync(path.join(home, 'public-pages.json')));
const archiveOrigins = mode === 'local' ? JSON.parse(fs.readFileSync(path.join(home, 'archive-origins.json'))) : {};
const repositoryPaths = mode === 'local' ? Object.entries(JSON.parse(fs.readFileSync(path.join(home, 'repository-paths.json')))).sort((a,b)=>b[0].length-a[0].length) : [];
const repositoryOrigins = mode === 'local' ? JSON.parse(fs.readFileSync(path.join(home, 'repository-origins.json'))) : {};
function currentPath(src) {
 const rel=path.relative(root,src).replaceAll(path.sep,'/');
 const match=repositoryPaths.find(([old])=>rel===old||rel.startsWith(old+'/'));
 return match ? path.join(root,match[1]+rel.slice(match[0].length)) : src;
}
const queue = [], mapped = new Map();
const write = (rel, text) => { const p = path.join(out, rel); fs.mkdirSync(path.dirname(p), {recursive:true}); generated.push(rel); const data=Buffer.from(text); if(!fs.existsSync(p)||!fs.readFileSync(p).equals(data)) fs.writeFileSync(p,data); };
const walk = d => fs.readdirSync(d, {withFileTypes:true}).flatMap(e => e.isDirectory() ? walk(path.join(d,e.name)) : e.name === '.DS_Store' ? [] : [path.join(d,e.name)]);
function register(src, rel) {
  if (fs.lstatSync(src).isSymbolicLink()) throw Error(`Symlink refused: ${src}`);
  if (mapped.has(src)) return mapped.get(src);
  mapped.set(src,rel); queue.push([src,rel]); return rel;
}
if(mode === 'public') {
 for(const rel of allow) {
  const src=path.join(doc,'public',rel);
  if(path.isAbsolute(rel)||rel.split('/').includes('..')||!fs.realpathSync(src).startsWith(fs.realpathSync(path.join(doc,'public'))+path.sep)) throw Error('Invalid public allowlist path: '+rel);
  register(src,rel);
 }
} else {
 for(const section of ['public','local']) {
  for(const src of walk(path.join(doc,section))) register(src,path.relative(doc,src).replace(/^public\//,'published/'));
 }
 register(path.join(doc,'README.md'),'README.md');
}
function destination(src) {
 src=currentPath(src);
 if(mode==='local') { const sources=JSON.parse(fs.readFileSync(path.join(home,'public-sources.json'))); const target=Object.entries(sources).find(([,original])=>path.join(root,original)===src); if(target) return 'published/'+target[0]; }
 if(mapped.has(src)) return mapped.get(src);
 if(mode === 'public') throw Error(`Public link outside allowlist: ${src}`);
 if(!src.startsWith(root + path.sep) || src.startsWith(path.join(root,'project/.local/archive')+path.sep)) throw Error(`Unsupported local target: ${src}`);
 if(!fs.existsSync(src)) {
   const old=path.relative(doc,src);
   if(legacy[old]) return destination(path.join(doc,legacy[old]));
   if(src === path.join(root,'AGENTS.md')) { const rel='source/missing-agents.md'; mapped.set(src,rel); write(rel,'# 既有引用：AGENTS.md\n\n该文件在本次实施之前已被本地删除。这里保留引用说明，不恢复或重建原文件。\n'); return rel; }
   throw Error(`Missing target: ${src}`);
 }
 const rel = 'source/' + path.relative(root,src);
 if(fs.statSync(src).isDirectory()) {
   const target = rel+'/index.md'; mapped.set(src,target);
   const entries = fs.readdirSync(src,{withFileTypes:true}).filter(e=>e.isDirectory());
   write(target, '# '+path.basename(src)+'\n\n本机目录索引；此处仅展示有 README 的子目录。\n\n'+entries.filter(e=>fs.existsSync(path.join(src,e.name,'README.md'))).map(e=>'- ['+e.name+'](/'+destination(path.join(src,e.name,'README.md'))+')').join('\n')+'\n');
   return target;
 }
 const sourceCode = /\.(rs|py|toml)$/.test(src) || (src.startsWith(path.join(root,'project')+path.sep) && /\.(js|css|html)$/.test(src));
 return register(src,sourceCode ? rel+(src.endsWith('.html')?'.source.md':'.md') : rel);
}
for(let i=0;i<queue.length;i++) {
 const [src,rel]=queue[i];
 if(!src.endsWith('.md')) {
   if(rel.endsWith('.md')) {const lang=path.extname(src).slice(1); write(rel,'# '+path.basename(src)+'\n\n本机源码快照，来自 `'+path.relative(root,src)+'`。\n\n````'+lang+'\n'+fs.readFileSync(src,'utf8')+'\n````\n');}
   else write('.static/'+rel,fs.readFileSync(src));
   continue;
 }
 let text = fs.readFileSync(src,'utf8');
 if(mode==='local' && rel==='local/index.md') text=text.replace('<!-- directory-tree -->',directoryMarkdown(path.join(doc,'local')));
 const origin = archiveOrigins[path.relative(doc,src)];
 const repositoryOrigin = repositoryOrigins[path.relative(root,src).replaceAll(path.sep,'/')];
 const linkBase = origin ? path.dirname(path.join(doc,origin)) : repositoryOrigin ? path.dirname(path.join(root,repositoryOrigin)) : path.dirname(src);
 // Rewrite Markdown destinations in generated copies only; canonical bodies remain intact.
 text=text.replace(/(!?\[(?:[^\[\]\n]|\[[^\]\n]*\])*\]\()([^\s)]+)(\))/g,(all,begin,url,end)=>{
  if(/^(?:[a-z][a-z0-9+.-]*:|#|\/\/)/i.test(url)) return all;
  const [p,hash]=url.split('#');
  if(!p) return all;
  const abs=path.resolve(linkBase,decodeURIComponent(p));
  const dest=destination(abs);
  return begin+'/'+dest+(hash?'#'+hash:'')+end;
 });
 // Vue must not interpret generic type placeholders in prose; code fences remain untouched.
 if(mode==='local' && rel.startsWith('local/') && !rel.endsWith('/index.md')) {
  const historical = /(?:archive\/|reference\/|design\/|evidence\/)/.test(rel) && !rel.includes('/0.1.0/') && rel !== 'local/design/architecture.md' && !rel.startsWith('local/design/modules/');
  if(historical) text='::: warning 历史资料与版本边界\n正文按原记录保留，可能包含旧 1.0.0、串口、TUI 或 WebUI 范围。当前公开版本为 0.1.2；请勿将这里的计划、设计或历史验收当成当前能力。\n:::\n\n'+text;
 }
 write(rel,text);
}
if(mode==='local') write('index.md',`# log_print 文档库\n\n在本机查阅公开使用说明、内部研发资料及历史归档。\n\n| 入口 | 内容 |\n|---|---|\n| [GitHub 文档](/published/index.md) | 面向使用者的 0.1.2 说明，作为独立公开站的唯一内容来源 |\n| [本地文档站](/local/index.md) | 按实际目录层级与相对路径浏览内部开发资料 |\n| [归档文档](/local/archive/index.md) | 历史计划与交付记录，完整保留 |\n\n搜索支持中文；顶部导航切换范围。历史文档的版本提示优先于正文中的“当前”等措辞。\n`);
fs.mkdirSync(path.join(out,'.vitepress/theme'),{recursive:true});
write('.vitepress/theme/index.js',fs.readFileSync(path.join(home,'theme.js')));
write('.vitepress/theme/Mermaid.vue',fs.readFileSync(path.join(home,'Mermaid.vue')));
write('.vitepress/theme/style.css',fs.readFileSync(path.join(home,'style.css')));
function localEntries(directory, base=path.join(doc,'local')) {
 const agentToolsDirectory=path.join(base,'research','log-print-agent-tools');
 return fs.readdirSync(directory,{withFileTypes:true})
  .filter(e=>e.name!=='.DS_Store' && !(directory===base && e.name==='archive') && !(directory===agentToolsDirectory && e.name==='README.md'))
  .sort((a,b)=>Number(b.isDirectory())-Number(a.isDirectory()) || a.name.localeCompare(b.name,'en'))
  .map(e=>{
    const full=path.join(directory,e.name), rel=path.relative(base,full).replaceAll(path.sep,'/');
    if(e.isDirectory()) return {text:e.name,collapsed:true,items:localEntries(full,base)};
    return {text:e.name,link:'/local/'+rel.replace(/\.md$/,'')};
  });
}
function directoryMarkdown(directory, base=directory, depth=0) {
 const agentToolsDirectory=path.join(base,'research','log-print-agent-tools');
 return fs.readdirSync(directory,{withFileTypes:true})
  .filter(e=>e.name!=='.DS_Store' && !(directory===base && ['archive','index.md'].includes(e.name)) && !(directory===agentToolsDirectory && e.name==='README.md'))
  .sort((a,b)=>Number(b.isDirectory())-Number(a.isDirectory()) || a.name.localeCompare(b.name,'en'))
  .map(e=>{
   const full=path.join(directory,e.name), rel=path.relative(base,full).replaceAll(path.sep,'/');
   return '  '.repeat(depth)+'- '+(e.isDirectory()?'**'+e.name+'**\n'+directoryMarkdown(full,base,depth+1):'['+rel+']('+rel+')');
  }).join('\n');
}
function sidebar(prefix) {
 const files=walk(out).filter(p=>p.endsWith('.md') && path.relative(out,p).startsWith(prefix));
 return files.map(p=>{const rel=path.relative(out,p).replaceAll(path.sep,'/');const title=rel.endsWith('/quickstart.md')||rel==='quickstart.md'?'快速开始':fs.readFileSync(p,'utf8').match(/^# (.+)$/m)?.[1]||path.basename(p,'.md'); return {text:title,link:'/'+rel.replace(/\.md$/,'')};});
}
const nav=mode==='local'?[{text:'GitHub 文档',link:'/published/index',activeMatch:'^/published/'},{text:'本地文档站',link:'/local/index',activeMatch:'^/local/(?!archive/)'},{text:'归档文档',link:'/local/archive/index',activeMatch:'^/local/archive/'}]:[{text:'使用说明',link:'/index'},{text:'GitHub',link:'https://github.com/TXyy2023/log_print'}];
function publicSidebar(prefix) {
 const groups=JSON.parse(fs.readFileSync(path.join(home,'public-navigation.json')));
 return groups.map(group=>({text:group.text,collapsed:group.text==='插件参考',items:group.items.map(item=>{
   if(!allow.includes(item.page)) throw Error('Navigation page outside public allowlist: '+item.page);
   return {text:item.text,link:'/'+prefix+item.page.replace(/\.md$/,'')};
 })}));
}
const bars=mode==='public'?publicSidebar(''):{'/published/':publicSidebar('published/'),'/local/archive/':sidebar('local/archive/'),'/local/':localEntries(path.join(doc,'local')),'/source/':[{text:'本地文档站',link:'/local/index'}]};
const config={head:[['link',{rel:'icon',href:'data:image/svg+xml,%3Csvg xmlns=%22http://www.w3.org/2000/svg%22 viewBox=%220 0 64 64%22%3E%3Crect width=%2264%22 height=%2264%22 rx=%2212%22 fill=%22%2328716b%22/%3E%3Ctext x=%2212%22 y=%2244%22 fill=%22white%22 font-size=%2238%22%3EL%3C/text%3E%3C/svg%3E'}]],lang:'zh-CN',title:mode==='local'?'log_print · 本地文档':'log_print',description:'日志采集与归档工具文档',outDir:path.join(home,'dist',mode),cleanUrls:false,themeConfig:{nav,sidebar:bars,outline:{level:[2,3],label:'本页目录'},docFooter:{prev:'上一页',next:'下一页'},search:{provider:'local',options:{locales:{root:{translations:{button:{buttonText:'搜索文档',buttonAriaLabel:'搜索文档'},modal:{noResultsText:'没有找到结果',resetButtonTitle:'清除搜索',footer:{selectText:'选择',navigateText:'切换',closeText:'关闭'}}}}}}}},vite:{publicDir:path.join(out,'.static'),server:{fs:{allow:[out,home]}}}};
write('.vitepress/config.mjs',`import {defineConfig} from 'vitepress';\nimport fs from 'node:fs';\nimport path from 'node:path';\nconst config=${JSON.stringify(config,null,2)};\nconfig.themeConfig.search.options.miniSearch={options:{tokenize:(text)=>Array.from(new Intl.Segmenter('zh',{granularity:'word'}).segment(text),s=>s.segment).filter(s=>/\\p{L}|\\p{N}/u.test(s))},searchOptions:{prefix:true,fuzzy:0.2}};\nconfig.ignoreDeadLinks=[(url)=>fs.existsSync(path.join(config.vite.publicDir,url.replace(/^\\//,'')))];\nconfig.markdown={config(md){const fence=md.renderer.rules.fence;md.renderer.rules.fence=(tokens,idx,options,env,self)=>tokens[idx].info.trim()==='mermaid'?'<Mermaid code="'+md.utils.escapeHtml(tokens[idx].content)+'" />':(tokens[idx].info=tokens[idx].info.replace('rust,no_run','rust'),fence(tokens,idx,options,env,self));}};\nexport default defineConfig(config);\n`);
write('build-inputs.json', JSON.stringify([...mapped.keys()].map(p=>path.relative(root,p)),null,2));
for(const rel of previous) if(!generated.includes(rel)) fs.rmSync(path.join(out,rel),{force:true});
fs.writeFileSync(path.join(out,'.generated.json'),JSON.stringify(generated));
console.log(`${mode}: ${mapped.size} inputs -> ${out}`);
