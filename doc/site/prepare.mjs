import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
const home = path.dirname(fileURLToPath(import.meta.url));
const doc = path.dirname(home), root = path.dirname(doc);
const mode = process.argv[2];
if (!['local', 'public'].includes(mode)) throw Error('Use local or public');
const base = mode === 'public' ? (process.env.DOCS_BASE || '/') : '/';
if (!/^\/(?:[A-Za-z0-9_~-]+(?:\.[A-Za-z0-9_~-]+)*\/)*$/.test(base)) throw Error('DOCS_BASE must be an absolute site path ending in /');
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
  for(const src of walk(path.join(doc,section))) register(src,path.relative(doc,src).replace(/^public\/zh\//,'zh/published/').replace(/^public\//,'published/'));
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
 write(rel,text);
}
if(mode==='local') {
 write('index.md',`# log_print documentation

English is the default language of the public manual. The local portal also includes private development notes in their original language.

| Entry | Contents |
|---|---|
| [Public manual](/published/index.md) | English user guides and reference |
| [简体中文手册](/zh/published/index.md) | Complete Chinese public manual |
| [Local documentation (中文)](/local/index.md) | Architecture, protocol, components, plans and validation |
| [Archive status (中文)](/local/archive/index.md) | Status of retired local material |

Internal navigation follows the actual directory tree. Private notes and source snapshots are never included in the public build.
`);
 write('zh/index.md',`# log_print 文档库

公开手册默认英语，可切换完整中文版本；本地开发资料保留原文。

| 入口 | 内容 |
|---|---|
| [中文公开手册](/zh/published/index.md) | 使用指南与参考 |
| [English manual](/published/index.md) | 默认英文公开手册 |
| [本地文档站](/local/index.md) | 架构、协议、组件、计划与验证 |
| [归档文档](/local/archive/index.md) | 过期资料清理状态 |
`);
}
fs.mkdirSync(path.join(out,'.vitepress/theme'),{recursive:true});
write('.vitepress/theme/index.js',fs.readFileSync(path.join(home,'theme.js')));
write('.vitepress/theme/Mermaid.vue',fs.readFileSync(path.join(home,'Mermaid.vue')));
write('.vitepress/theme/style.css',fs.readFileSync(path.join(home,'style.css')));
function localEntries(directory, base=path.join(doc,'local')) {
 return fs.readdirSync(directory,{withFileTypes:true})
  .filter(e=>(e.isDirectory() ? walk(path.join(directory,e.name)).some(p=>p.endsWith('.md')) : e.name.endsWith('.md')) && !(directory===base && e.name==='archive'))
  .sort((a,b)=>Number(b.isDirectory())-Number(a.isDirectory()) || a.name.localeCompare(b.name,'en'))
  .map(e=>{
    const full=path.join(directory,e.name), rel=path.relative(base,full).replaceAll(path.sep,'/');
    if(e.isDirectory()) return {text:e.name,collapsed:true,items:localEntries(full,base)};
    return {text:path.basename(e.name,'.md'),link:'/local/'+rel.replace(/\.md$/,'')};
  });
}
function directoryMarkdown(directory, base=directory, depth=0) {
 return fs.readdirSync(directory,{withFileTypes:true})
  .filter(e=>(e.isDirectory() ? walk(path.join(directory,e.name)).some(p=>p.endsWith('.md')) : e.name.endsWith('.md')) && !(directory===base && ['archive','index.md'].includes(e.name)))
  .sort((a,b)=>Number(b.isDirectory())-Number(a.isDirectory()) || a.name.localeCompare(b.name,'en'))
  .map(e=>{
   const full=path.join(directory,e.name), rel=path.relative(base,full).replaceAll(path.sep,'/');
   return '  '.repeat(depth)+'- '+(e.isDirectory()?'**'+e.name+'**\n'+directoryMarkdown(full,base,depth+1):'['+path.basename(e.name,'.md')+']('+rel+')');
  }).join('\n');
}
function sidebar(prefix) {
 const files=walk(out).filter(p=>p.endsWith('.md') && path.relative(out,p).startsWith(prefix));
 return files.map(p=>{const rel=path.relative(out,p).replaceAll(path.sep,'/');const title=rel.endsWith('/quickstart.md')||rel==='quickstart.md'?'快速开始':fs.readFileSync(p,'utf8').match(/^# (.+)$/m)?.[1]||path.basename(p,'.md'); return {text:title,link:'/'+rel.replace(/\.md$/,'')};});
}
function publicSidebar(prefix, language) {
 const groups=JSON.parse(fs.readFileSync(path.join(home,'public-navigation.json')));
 return groups.map((group,index)=>({text:group.text[language],collapsed:index===3,items:group.items.map(item=>{
   const source=(language==='zh'?'zh/':'')+item.page;
   if(!allow.includes(source)) throw Error('Navigation page outside public allowlist: '+source);
   return {text:item.text[language],link:'/'+prefix+item.page.replace(/(^|\/)index\.md$/,'$1').replace(/\.md$/,'')};
 })}));
}
const internalBars=mode==='local'?{'/local/archive/':sidebar('local/archive/'),'/local/':localEntries(path.join(doc,'local')),'/source/':[{text:'Local documentation (中文)',link:'/local/index'}]}:{};
const zhSearch={translations:{button:{buttonText:'搜索文档',buttonAriaLabel:'搜索文档'},modal:{displayDetails:'显示详细列表',resetButtonTitle:'清除搜索',backButtonTitle:'关闭搜索',noResultsText:'没有找到结果',footer:{selectText:'选择',selectKeyAriaLabel:'回车',navigateText:'切换',navigateUpKeyAriaLabel:'向上',navigateDownKeyAriaLabel:'向下',closeText:'关闭',closeKeyAriaLabel:'Esc'}}}};
function locale(language) {
 const zh=language==='zh';
 const prefix=mode==='public'?(zh?'zh/':''):(zh?'zh/published/':'published/');
 const nav=[{text:zh?'使用手册':'Manual',link:'/'+prefix}];
 if(mode==='local') nav.push({text:zh?'本地文档站':'Local notes (中文)',link:'/local/index',activeMatch:'^/local/(?!archive/)'},{text:zh?'归档文档':'Archive (中文)',link:'/local/archive/index',activeMatch:'^/local/archive/'});
 nav.push({text:'GitHub',link:'https://github.com/TXyy2023/log_print'});
 return {label:zh?'简体中文':'English',lang:zh?'zh-CN':'en',description:zh?'本地日志采集、工作台与归档使用手册':'Local log collection, workbenches and archiving',themeConfig:{nav,sidebar:mode==='public'?publicSidebar(prefix,language):{...internalBars,'/published/':publicSidebar('published/','en'),'/zh/published/':publicSidebar('zh/published/','zh')},
  // Local-only notes have no translated counterpart; the menu returns to each portal.
  i18nRouting:mode==='public',
  outline:{level:[2,3],label:zh?'本页目录':'On this page'},docFooter:{prev:zh?'上一页':'Previous page',next:zh?'下一页':'Next page'},
  langMenuLabel:zh?'切换语言':'Change language',sidebarMenuLabel:zh?'菜单':'Menu',returnToTopLabel:zh?'返回顶部':'Return to top',skipToContentLabel:zh?'跳转到正文':'Skip to content',
  darkModeSwitchLabel:zh?'外观':'Appearance',lightModeSwitchTitle:zh?'切换为浅色主题':'Switch to light theme',darkModeSwitchTitle:zh?'切换为深色主题':'Switch to dark theme',
  notFound:zh?{title:'页面不存在',quote:'请检查地址或返回手册首页。',linkLabel:'返回首页',linkText:'返回首页'}:{title:'PAGE NOT FOUND',quote:'Check the address or return to the manual.',linkLabel:'Go to home',linkText:'Go to home'}
 }};
}
const brandingPrefix=mode==='public'?'':'published/';
const brandIcon='/'+brandingPrefix+'assets/branding/log-print-icon.svg';
const config={base,head:[['link',{rel:'icon',type:'image/svg+xml',href:base+brandIcon.slice(1)}]],lang:'en',title:mode==='local'?'log_print · Local docs':'log_print',description:'Local log collection, workbenches and archiving',locales:{root:locale('en'),zh:locale('zh')},outDir:path.join(home,'dist',mode),cleanUrls:false,themeConfig:{logo:brandIcon,search:{provider:'local',options:{locales:{zh:zhSearch}}}},vite:{publicDir:path.join(out,'.static'),server:{fs:{allow:[out,home]}}}};
write('.vitepress/config.mjs',`import {defineConfig} from 'vitepress';\nimport fs from 'node:fs';\nimport path from 'node:path';\nconst config=${JSON.stringify(config,null,2)};\nconfig.themeConfig.search.options.miniSearch={options:{tokenize:(text)=>Array.from(new Intl.Segmenter('zh',{granularity:'word'}).segment(text),s=>s.segment).filter(s=>/\\p{L}|\\p{N}/u.test(s))},searchOptions:{prefix:true,fuzzy:0.2}};\nconfig.ignoreDeadLinks=[(url)=>fs.existsSync(path.join(config.vite.publicDir,url.replace(/^\\//,'')))];\nconfig.markdown={config(md){const fence=md.renderer.rules.fence;md.renderer.rules.fence=(tokens,idx,options,env,self)=>tokens[idx].info.trim()==='mermaid'?'<Mermaid code="'+md.utils.escapeHtml(tokens[idx].content)+'" />':(tokens[idx].info=tokens[idx].info.replace('rust,no_run','rust'),fence(tokens,idx,options,env,self));}};\nexport default defineConfig(config);\n`);
write('build-inputs.json', JSON.stringify([...mapped.keys()].map(p=>path.relative(root,p)),null,2));
for(const rel of previous) if(!generated.includes(rel)) fs.rmSync(path.join(out,rel),{force:true});
fs.writeFileSync(path.join(out,'.generated.json'),JSON.stringify(generated));
console.log(`${mode}: ${mapped.size} inputs -> ${out}`);
