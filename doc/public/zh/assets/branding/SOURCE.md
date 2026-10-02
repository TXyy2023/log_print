# 主 Logo 来源 {#primary-logo-provenance}

- 用户于 2026-10-03 选定负形 LP 方案（候选 6）。
- `log-print-logo.png`：用户所选 1254 × 1254 图片，从附件原样复制。
- `log-print-icon.svg`：内嵌同一 PNG，通过 SVG 视窗仅展示 LP 图标；没有重新生成或修饰像素。
- 用于中英文 README、文档首页/导航栏/浏览器图标、WebUI 页头/浏览器图标。
- 原设计由内置 imagegen 生成，不宣称具体模型。
- README 透明版本：`log-print-logo-transparent-light.svg` 与 `log-print-logo-transparent-dark.svg`。两版均内嵌原始图片，以同一 SVG 亮度遮罩生成透明度，缩小画布留白，并根据阅读者的主题切换前景颜色；白底与 LP 镂空区域透明。imagegen 抠图候选因遮罩杂点未采用。
- 下方早期横幅继续保留为历史素材。

## 选定 Logo 提示词 {#selected-logo-prompt}

```text
Use case: logo-brand
Asset type: original logo concept for "log_print", a local logging and log-stream developer utility.
Design an exceptionally refined contemporary software identity. This is a new direction after rejected bulky complicated terminal illustrations. Treat it as professional graphic identity work: disciplined geometry, visual wit, beautiful negative space, restrained detail, precise optical balance. Flat solid fills, perfectly clean edges. White square presentation canvas with abundant breathing room. One standalone emblem above, one modest-size wordmark below; do not repeat the emblem. Text exactly "log_print", lowercase with underscore, impeccable kerning, light or medium-weight type, never heavy bubbly bold. Choose a restrained palette suitable to this specific concept. No explanatory text, no extra words, no numbers, no mockups, no gradients, textures, shading, embossing, 3D, watermark. Avoid generic terminal windows, >_ motifs, multicolor cable networks, connected colored dots, oversized rounded strokes. The mark must be compelling as a small software icon.
Primary request: A bold compact single-color geometric tile using expertly carved negative space to hint subtly at both lowercase l and p. Angular cuts and carefully controlled rounded counters, exquisite balance, no circuitry or wiring. A restrained modern grotesk wordmark beneath. Identity should feel engineered and premium.
```

<span id="banner-provenance"></span>

# 标题图片来源

- 文件：`log-print-banner.png`，2172 × 724 PNG。
- 生成日期：2026-09-22。
- 方式：Codex 内置 imagegen 工具。图片为本项目原创生成标识，原样复制到仓库。
- 用户指定 `gpt-image-2.5`；工具没有可选模型参数或可核验的模型回执，因此不宣称已确认使用该模型。
- 设计：多路日志经终端汇聚再输出，深紫底色、琥珀色与淡紫色；未使用其他项目的官方标识。

<span id="final-prompt"></span>

## 最终提示词

```text
Use case: logo-brand. Asset type: finished wide GitHub README title banner for the independent open-source Rust developer tool named exactly "log_print". Create an original polished visual identity: a compact geometric emblem made of three parallel log-line streams flowing through a central small terminal window and branching into a neat output stack. The emblem should suggest log collection, routing and readable output. Large crisp wordmark "log_print", exactly lowercase with one underscore, placed beside the emblem. One small subtitle exactly "Local logs. Clear signals." A tiny label exactly "RUST · CLI · AGENT SKILL". Wide horizontal 3:1 composition with generous safe margins and negative space, restrained deep ink-purple background, warm amber primary accent and soft periwinkle secondary accent, bright near-white wordmark. Elegant flat vector-like shapes rendered as a high-quality raster graphic, sharp readable typography, carefully balanced spacing. This is a compact developer-tool identity, no photographic objects, no robot, no printer hardware, no stock clip art, no official Rust/GitHub/OpenAI logos, no glossy 3D, no neon glow, no watermark. Deliver one finished banner.
```
