# 真实软件日志测试仓库候选

> 调研日期：2026-09-17。本文档只是候选清单和后续落地约束；当前没有克隆第三方源码、没有拉取镜像、没有启动容器，也没有人工编造日志。

## 目录定位

`project/workloads/` 用于收纳“真实软件工作负载”的容器配方、上游版本信息、操作脚本和实测证据。目录名中的 `mock` 不代表日志可以伪造：正式样本必须来自正在运行的上游软件。

它与现有 CI/CD 资产的职责不同：

- `tests/`、`quality/ci/` 和 `.github/workflows/` 验证 `log_print` 本身的功能完整性与回归。
- `project/workloads/` 向 `log_print` 提供外部真实软件的 stdout、stderr、访问日志、错误日志和持久日志，用于发现真实日志格式与运行时边界问题。
- 此目录不参与默认发布验收；日后只有明确标记的工作负载才可进入可选的真实软件验收。

## Apple Container 基线与边界

- Apple [`container`](https://github.com/apple/container) 读写 OCI 兼容镜像，适合在 Apple Silicon Mac 上运行 Linux/arm64 工作负载。
- 本机于 2026-09-17 发现 `container` CLI 1.0.0；它的帮助面提供 `build`、`run` 和 `logs --follow`，但未列出 Compose 子命令。因此第一批优先单容器，Compose 项目需在后续明确翻译为 network、volume 和多个 `container run`。
- 下表中的“单容器”只表示上游有清晰的单容器路径，不代表已在本机通过。最终必须实际完成 `linux/arm64` 拉取/构建→启动→健康检查→真实请求→日志采集→停止与清理，才能标记为“Apple Container 可用”。
- 所有网络端口默认只绑定 `127.0.0.1`；使用一次性 volume 和专用测试账户，不使用真实凭据、个人数据或公网业务。

## 第一批：优先落地的单容器项目

| 顺序 | 语言 / 项目 | 上游与容器入口 | 可获得的真实日志面 | 适合检查 `log_print` 的内容 | 当前注意点 |
| --- | --- | --- | --- | --- | --- |
| 1 | C / NGINX | [源码](https://github.com/nginx/nginx) · [官方 Dockerfiles](https://github.com/nginx/docker-nginx) | access log、error log、启停和配置错误 | 纯文本行、2xx/4xx/5xx、stdout/stderr、短时高流量、长连接 | 最适合先建基线；需锁定具体版本和 arm64 镜像 digest |
| 2 | C# / ASP.NET Core Razor | [.NET 官方样例](https://github.com/dotnet/dotnet-docker/tree/main/samples/AspNetCoreRazorApp) | Hosting 启停、HTTP 请求、类别名、Event ID、异常 | 带缩进的多行文本、等级解析、请求关联、优雅停止 | 使用样例子目录构建；不把整个镜像仓库当成单一应用 |
| 3 | Rust / Meilisearch | [meilisearch/meilisearch](https://github.com/meilisearch/meilisearch) | 启动配置、HTTP API、索引和异步任务、告警/错误 | Rust 服务日志、任务状态变化、连续流、持久化后重启 | 数据目录使用一次性 volume；关闭非必要遥测并记录配置 |
| 4 | Go / Gitea | [go-gitea/gitea](https://github.com/go-gitea/gitea) | 启动、数据库迁移、HTTP 访问、Git 操作、后台任务 | 长时运行、多类别、访问+应用混合日志、重启连续性 | 优先 SQLite 单容器模式；只创建本地一次性账户和仓库 |
| 5 | JavaScript / Uptime Kuma | [louislam/uptime-kuma](https://github.com/louislam/uptime-kuma) | 启动、监控调度、WebSocket、SQLite、连接失败 | 周期性背景日志、间歇错误、长时运行和无请求时段 | 需一次本地 UI 初始化；监控目标只能是本次测试自己的本地服务 |
| 6 | Java / Keycloak | [keycloak/keycloak](https://github.com/keycloak/keycloak) · [官方容器说明](https://www.keycloak.org/server/containers) | JVM/Quarkus 启动、认证、安全事件、健康检查、优雅停止 | 多行日志、线程/类别前缀、安全事件、较大启动峰值 | 只用 `start-dev` 做本地测试；设内存上限，访问/事件日志需显式配置 |
| 7 | Python / httpbin | [psf/httpbin](https://github.com/psf/httpbin) | Flask/Gunicorn 启动、访问和异常 | 多 HTTP 方法、状态码、延迟、大响应、请求元数据 | 仓库有 Dockerfile，但默认 `httpbin.bash` 未开启 Gunicorn access log；落地时需用经验证的参数开启，不能把“没日志”误判为采集失败 |
| 8 | C++ / Oat++ example-crud | [oatpp/example-crud](https://github.com/oatpp/example-crud) | HTTP/CRUD、SQLite、Swagger、运行错误 | C++ 服务日志、API 正常/异常路径、本地数据库操作 | 上游更新频率较低，Dockerfile 依赖浮动的 `alpine:latest` 和构建期下载；需先做供应链固定和 arm64 构建验证 |

### 建议的首轮顺序

1. NGINX：建立最简单的访问/错误日志基线。
2. ASP.NET Core Razor：检查多行、分类前缀和结构化解析。
3. Meilisearch：检查异步任务、持续日志和重启。
4. Gitea：进入完整应用的长时、多类别、文件/控制台混合场景。

这四个通过后，再补 Uptime Kuma、Keycloak、httpbin 和 Oat++，可以减少一开始就同时排查多种容器兼容问题的概率。

## 第二批：高负载或多容器项目

| 语言 / 项目 | 上游入口 | 增加的覆盖 | 为什么不放第一批 |
| --- | --- | --- | --- |
| C / Redis | [源码](https://github.com/redis/redis) · [官方镜像 Dockerfiles](https://github.com/docker-library/redis) | 启停、RDB/AOF、内存和持久化警告 | 默认不记录每条命令，更适合低噪声和生命周期场景 |
| C++ / ClickHouse | [ClickHouse/ClickHouse](https://github.com/ClickHouse/ClickHouse) | 高吞吐、查询、后台合并、多文件日志、轮转 | 镜像、内存和磁盘负担更大，需先确定资源上限 |
| Java / Spring Petclinic | [spring-projects/spring-petclinic](https://github.com/spring-projects/spring-petclinic) | Spring Boot、H2、Web/MVC、数据库配置错误 | 上游明确没有 Dockerfile，`spring-boot:build-image` 假定 Docker daemon；需先设计 Apple Container 可构建的 Containerfile |
| Python + TypeScript / FastAPI full-stack | [fastapi/full-stack-fastapi-template](https://github.com/fastapi/full-stack-fastapi-template) | API、前端、PostgreSQL、Traefik、迁移与启动顺序 | 主路径是 Compose，需翻译多容器网络与依赖 |
| Python / NetBox | [netbox-community/netbox-docker](https://github.com/netbox-community/netbox-docker) | Django/Gunicorn、worker、PostgreSQL、Redis、任务队列 | 多服务、初始化慢，资源和日志源较多 |
| Ruby / Chatwoot | [chatwoot/chatwoot](https://github.com/chatwoot/chatwoot) | Rails、Sidekiq、PostgreSQL、Redis、WebSocket、后台任务 | 依赖链长，需翻译 Compose，还需安全地处理本地初始账户 |
| PHP / Nextcloud | [应用源码](https://github.com/nextcloud/server) · [容器配方](https://github.com/nextcloud/docker) | Apache/FPM、PHP、数据库、应用文件日志、cron | 上游容器仓库自身定位为微服务/专家部署，日志分布在控制台和文件 |
| Elixir / Plausible | [应用源码](https://github.com/plausible/analytics) · [Community Edition](https://github.com/plausible/community-edition) | Phoenix/BEAM、PostgreSQL、ClickHouse、后台任务 | 是多容器 Compose 工作负载，资源与启动顺序都更复杂 |
| Swift / Vapor + PostgreSQL | [vapor/template-fluent-postgres](https://github.com/vapor/template-fluent-postgres) | Linux Swift、Vapor HTTP、Fluent ORM、PostgreSQL | 需 PostgreSQL 且构建镜像较大；适合在多容器基础成熟后补语言覆盖 |

## 本轮不建议采用的候选

- [`maybe-finance/maybe`](https://github.com/maybe-finance/maybe)：GitHub 于本次调研时标记为 archived，不适合新增为长期测试依赖。
- [`spring-petclinic/spring-petclinic-kotlin`](https://github.com/spring-petclinic/spring-petclinic-kotlin)：有 Dockerfile，但当前文件仍使用 Gradle 4.7/JDK 8 时代的基础镜像与 JVM 参数，先不作为 Kotlin 基线。
- [`vapor/template`](https://github.com/vapor/template)：Dockerfile 包含 `{{name}}` 模板占位符，不是克隆后可直接构建的完整应用；使用上表已展开的 `template-fluent-postgres` 更合适。

## 真实日志的生成规则

允许的日志来源：

- 软件自身的 stdout/stderr。
- 软件自身写入的 access/error/application 日志文件。
- 容器运行时返回的该容器 stdio 日志。
- 对该软件发出可追溯的真实本地 HTTP/API/数据库操作后，由软件自身产生的日志。

不允许把手写文本、随机行、事后编辑的“像日志”文件或修改过的采集结果冒充为软件原始日志。负载请求可以由测试脚本发出，但必须记录请求脚本、时间段、容器版本和预期行为，不能直接修改日志。

## 与 `log_print` 的接入方向

第一阶段不新建语言专用插件，优先复用现有通用输入：

```text
软件容器 stdout/stderr
  → container logs --follow <container>
  → input-program
  → Core 原始流
  → output-raw / output-transform / output-tui / output-webui
```

如果软件只写日志文件，则把一次性日志目录挂载到主机，再由 `input-file` 跟随。正式验收前要单独确认 Apple `container logs` 是否保留或合并容器的 stdout/stderr；若合并，必须在证据中明确标注，不虚构原始通道归属。

## 每个工作负载的落地验收

后续真正加入项目时，每个工作负载都应留下：

1. 上游仓库 URL、精确 commit/tag、当时的 License 和镜像 digest；不在可重现验收中使用浮动的 `latest`。
2. 实际 `linux/arm64` 拉取或构建结果，以及 Apple Container 版本、端口、CPU/内存上限和 volume 定义。
3. 健康检查、正常请求、可预期的非法请求、优雅停止与再启动证据。
4. 一份可重放的本地负载脚本，明确它做了什么；不访问未授权的外部服务。
5. `log_print` 原始流的字节数、哈希、时间范围、丢失/gap 状态、stdout/stderr 边界和停止后的清理状态。
6. 与容器直接日志的对照：不只验证进程存活或 HTTP 200，还要确认预期日志确实进入、可读、可保存且无未解释丢失。

## 建议的后续目录结构

当开始落地第一个项目时再创建下列子目录，本轮暂不创建空骨架：

```text
project/workloads/
├── README.md
├── workloads/
│   └── <language>-<project>/
│       ├── SOURCE.md          # 仓库、commit/tag、License、digest
│       ├── Containerfile      # 仅在上游路径不够时增加
│       ├── run.sh             # 启动/停止，只回收自己创建的资源
│       ├── verify.sh          # 健康、负载、日志和清理验证
│       └── log-print.json     # input-program/input-file 接入
├── upstream/                       # 可选的本地浅克隆，先决定是否忽略或使用 submodule
└── evidence/                       # 本地验收产物，是否入库需另行决策
```

## 下一步建议

先只落地 NGINX：锁定一个可用的 linux/arm64 版本与 digest，创建最小 Apple Container 运行脚本，用真实 2xx/404/5xx 请求验证 `container logs --follow → input-program → Core → output-raw`。它是最小但足够有代表性的第一个闭环。
