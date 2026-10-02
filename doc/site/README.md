# 文档站维护

站点使用仓库锁定的 VitePress 与 Mermaid 依赖。需要 Node.js 20.19+ 或兼容的更新 LTS、npm；服务仅监听 127.0.0.1。

## 启动与停止

从仓库根目录运行：

```sh
npm ci --prefix doc/site
npm start --prefix doc/site
npm run status --prefix doc/site
npm run stop --prefix doc/site
```

本机地址 http://127.0.0.1:5173/ 。后台服务退出终端后继续运行；重启电脑后需要重新 start。日志为 `doc/site/.cache/service.log`。端口已占用时拒绝启动，不更换端口或停止其他服务。

前台运行用 `npm run dev:local --prefix doc/site`，Ctrl-C 停止。修改规范正文后自动更新网页；生成缓存与产物不可手工维护。

## 正文与入口

| 本机入口 | 正文位置 | 内容 |
| --- | --- | --- |
| GitHub 文档 | `doc/public/` | 公开使用手册、任务指南、CLI/配置和五个插件参考 |
| 本地文档站 | `doc/local/` | 当前架构、协议、SDK、组件、用户计划记录与验证入口 |
| 归档文档 | `doc/local/archive/index.md` | 清理状态入口；不再发布过期正文或附件 |

公开手册是使用参数与命令的单一来源，README 和内部开发页通过链接引用。内部技术文档按当前源码维护。`doc/local/plans/` 由用户维护，旧计划合理保留，不因版本变化删除、重写或合并。旧实现、被替代的方案和过期验收从站点删除，不靠历史警告继续展示。

内部导航按实际目录生成，一级菜单为文件夹名，文档名只显示文件名（不含目录前缀和 `.md` 后缀），链接仍使用真实相对路径。只将 Markdown 正文列入页面导航，空目录不生成菜单。旧链接映射只保留最终目标仍存在的条目。

本机 URL 为 `/published/`、`/local/`、`/local/archive/`；`published` 避免与 VitePress 静态资源目录重名。两站提供中文全文搜索，Mermaid 在浏览器按需渲染。

## 新增、删除与验证

公开页面需要同时检查 `public-pages.json` 白名单和 `public-navigation.json` 导航。`public-sources.json` 将本机源码 README 链接路由到公开正文，不复制正文。

删除页面时同步清理正文引用、映射、导航和无用途附件，然后重新生成和构建。恢复快照放在 `quality/artifacts/`，不能再纳入站点正文或搜索。内部资料、依赖、缓存、产物和本地快照保持 Git 忽略范围。

```sh
npm run build:local --prefix doc/site
npm run build:public --prefix doc/site
python3 quality/tests/docs/verify_links.py doc/site/dist/local
python3 quality/tests/docs/verify_links.py doc/site/dist/public
```

本机产物 `doc/site/dist/local/` 包含内部资料，公开产物 `doc/site/dist/public/` 只接受公开白名单。公开构建拒绝越界链接、路径逃逸及软链接，并自动审查输入范围；不能将本机产物发布。两种模式有独立缓存、配置、搜索和产物。

`prepare.mjs` 在缓存中复制正文、改写链接，必要时将当前源码呈现为代码页面。`--refresh` 删除已移除的生成输入；正式构建重新生成全部缓存和产物。不能手改 `.cache/` 或用隐藏导航代替公开内容隔离。

## GitHub Pages

公开站：[在线使用手册](https://TXyy2023.github.io/log_print/)。部署配置见 `.github/workflows/docs.yml`：dev/main 推送和手动触发进行文档验证，仅 main 部署；本次本地整理不会自动提交或部署。

工作流以 `/log_print/` 为站点路径，本机内部站始终为 `/`。复现公开部署构建：

```sh
DOCS_BASE=/log_print/ npm run build:public --prefix doc/site
python3 quality/tests/docs/verify_links.py doc/site/dist/public --base /log_print/
npm run preview:public --prefix doc/site
```

预览 http://127.0.0.1:5174/log_print/ 。恢复根路径预览需重新执行不带 DOCS_BASE 的公开构建。GitHub Pages 的构建和部署结果需从实际工作流确认，本地构建通过不表示线上已更新。
