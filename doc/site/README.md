# log_print 文档网站

VitePress 1.6.4；Node.js 20.19+ 或受支持的更新 LTS、npm。本机站点仅监听 127.0.0.1，不需要数据库或远端服务。

在仓库根目录运行：

```sh
npm ci --prefix doc/site
npm start --prefix doc/site
npm run status --prefix doc/site
# 停止本工具启动的文档进程
npm run stop --prefix doc/site
```

打开 http://127.0.0.1:5173/ 。后台服务退出当前终端后继续运行；重启电脑后需要重新 start。日志在 `doc/site/.cache/service.log`。端口占用时拒绝启动，不切换或停止其他服务。

前台运行：`npm run dev:local --prefix doc/site`，Ctrl-C 停止。修改 `doc/public/`、`doc/local/` 下正文后自动更新网页；正文扫描和递归监听不包含 `doc/site/`，避免读入依赖、缓存和产物。生成目录不可手工维护。

## 单一正文来源与三个入口

| 本机入口 | 编辑位置 | 页面范围 |
|---|---|---|
| 公开文档 | `doc/public/` | 入门、任务指南、CLI/配置及六个插件参考 |
| 内部开发文档 | `doc/local/`（归档子目录以外） | 参考、设计、计划、研究、验收和图表 |
| 归档文档 | `doc/local/archive/` | 旧计划、历史发布和交付记录 |

公开手册正文统一维护在 `doc/public/`，两站均读取此处。根 README 和六个插件 README 仅保留仓库入口与链接，不再复制正式手册正文。`public-sources.json` 仅用于本机旧源码链接的路由映射，不是内容同步清单。`public-navigation.json` 显式维护侧栏顺序，`public-pages.json` 维护公开构建白名单；新增页面同步更新二者。

本机 URL 使用 `/published/`（避免与 VitePress 的保留静态资源目录 `public` 混淆）、`/local/`、`/local/archive/`。内部混合资料保留原分类并提示版本边界，不因存在未来设计就整体移入归档。旧 1.0.0 叙述不表示当前 0.1.1 能力。

## 构建与隔离

```sh
npm run build:local --prefix doc/site
npm run build:public --prefix doc/site
npm run preview:public --prefix doc/site
```

本机产物 `doc/site/dist/local/` 含内部资料，不能发布。公开产物 `doc/site/dist/public/` 只来自 `public-pages.json` 显式清单；新增公开文档或资源须更新清单。构建拒绝越界引用、路径逃逸和软链接指向内部。公开构建后自动审查输入列表及内部文件名标记。两种构建有独立缓存、配置、搜索索引和产物。

`prepare.mjs` 将规范源复制到独立缓存并改写站内链接，原文不受这些构建适配影响。内部站源码引用会呈现为带代码块的页面，原始数据和 SVG 作为本机静态附件；已有 AGENTS.md 删除保持，只给缺失引用提示。目录迁移前的旧引用使用 `legacy-paths.json` 解析。

两个站都使用中文分词的本地全文搜索。Mermaid 按需在浏览器渲染；原有 19 张 SVG 和 Markdown 中的 Mermaid 源码均保留。代码块、图表及下载附件依然属于原文件所写的历史环境。

`doc/public/`、`doc/local/` 及文档入口继续默认 Git 忽略；`doc/site/` 的工具源码可单独纳入跟踪，依赖、缓存和产物仍忽略。本轮不提交、推送或发布；未来公开时只逐项选择公开正文及必要构建文件，不能整体加入 doc。构建工具不会自动创建仓库或部署。

## 备份和回滚

- 初次整理前：`quality/artifacts/doc-reorganization-2026-09-19/original-doc/`，103 文件，与同目录 `source-manifest.json` SHA-256 全部相符。
- 本轮明确批准实施前：`quality/artifacts/vitepress-approved-20260919-224610/`，含当时 doc、脚手架配置和 `.gitignore`。
- 早期接手快照：`quality/artifacts/vitepress-2026-09-19/`。

回滚前先停止本站，再另存当时整个 doc、doc/site（依赖可排除）和 .gitignore，之后选择对应快照恢复。不要直接覆盖后续新增正文。初次整理的旧 rollback.py 有哈希保护，会拒绝覆盖本轮改动；这是预期行为。备份含内部资料，仅本地保存。

## 本次公开手册整理

公开侧栏按入门、使用指南、参考、插件参考组织，共20页；内部入口与侧栏按真实目录嵌套，一级菜单为文件夹名，文档显示相对于 `doc/local/` 的实际路径，归档独立。修改前快照位于 `quality/artifacts/public-manual-20260919-225737/`，包括旧公开正文、README、站点配置及内部入口。其他内部正文未作改写。

源码 README 已改为指向单一正文的入口；将来提交时，需将这些入口与其引用的公开正文一起逐项纳入跟踪，避免远端链接缺失。当前不执行提交或发布。

## 三入口与移除文档

本机顶部为“GitHub 文档 / 本地文档站 / 归档文档”。GitHub 文档是待公开手册的本机入口，不表示已部署 GitHub Pages。本地导航由磁盘目录生成，归档不混入内部菜单。`doc/local/index.md` 的目录标记在生成时展开，新增、移动文档后自动更新。

2026-09-19 的移除资料存入 `doc/local/archive/removed-2026-09-19/`，清单记录来源和哈希。移动后的旧链接由 `legacy-paths.json` 转到新位置；归档原文的相对链接根据 `archive-origins.json` 按原位置解析，保留原文字节。今后移除资料应先移入归档并登记原路径，不直接删除。

## 项目目录迁移

站点工具已从 `docs-site/` 移到 `doc/site/`。源码及测试旧路径由 `repository-paths.json` 映射；`repository-origins.json` 为完整保留的插件历史报告指定原始链接基准目录。这些映射仅用于本地站，公开构建仍只接受公开白名单。

链接检查现位于 `quality/tests/docs/verify_links.py`，可分别传入 `doc/site/dist/local` 和 `doc/site/dist/public`。构建与验收命令见 [测试说明](../../quality/README.md)。
