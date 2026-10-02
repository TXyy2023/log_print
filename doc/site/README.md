# Documentation site maintenance

The site uses the locked VitePress and Mermaid dependencies. Use Node.js 20.19+ or a compatible newer LTS and npm; CI uses Node.js 24. Local services listen only on 127.0.0.1.

## Start and stop

From the repository root:

```sh
npm ci --prefix doc/site
npm start --prefix doc/site
npm run status --prefix doc/site
npm run stop --prefix doc/site
```

The local portal is http://127.0.0.1:5173/. The background service survives closing the terminal; start it again after reboot. Logs are in `doc/site/.cache/service.log`. If the port is occupied, startup refuses without changing ports or stopping another service.

Use `npm run dev:local --prefix doc/site` for a foreground service, stopped with Ctrl-C. Source edits regenerate pages automatically. Do not edit generated caches or build output.

## Sources and languages

English is the default for the repository README, public manual and local portal. The public manual has a complete Simplified Chinese translation. English public URLs retain the existing root paths; Chinese pages live under `/zh/`. The VitePress language menu switches to the equivalent public page, without browser-language redirects.

| Source | Public URL | Local portal URL |
| --- | --- | --- |
| `doc/public/*.md` and subdirectories, excluding zh | `/` | `/published/` |
| `doc/public/zh/` | `/zh/` | `/zh/published/` |
| Shared public images in `doc/public/assets/` | `/assets/` | `/published/assets/` |
| `doc/local/` | Never published | `/local/` |
| `doc/local/archive/index.md` | Never published | `/local/archive/` |

`README.md` is the English repository entry; `README.zh-CN.md` is its Chinese counterpart. Public manuals are the source of command and option documentation. Internal notes and user-owned plans keep their original language and paths. In the local portal, the language menu returns to the English or Chinese portal because private notes have no translated equivalents. Each portal links to its language's complete manual.

Public search uses separate English and Chinese indexes. Word segmentation supports both English and Chinese. Local search includes only generated pages and may also include original-language private notes. Mermaid diagrams render on demand in the browser.

## Add or update a translation

1. Update the English body under `doc/public/` and the matching relative path under `doc/public/zh/`. Preserve command names, option names and identifiers. Translate explanations and sample display labels; keep documented behavior consistent. Paired headings retain alias anchors from the other language so language switches and existing Chinese deep links keep working; preserve these aliases when editing.
2. Add both Markdown files to `public-pages.json`. Shared assets are allowlisted once; Chinese pages use relative links to the shared assets.
3. Add the page once to `public-navigation.json`, providing both `text.en` and `text.zh` labels. The generator builds each locale's navigation from that entry.
4. Update both repository README entries if visible capabilities change. Update `public-sources.json` when a repository README should route to a manual page in the local build.
5. Run all checks below. Remove obsolete references, mappings and unused attachments when removing a page.

For an additional language, add a matching source tree and allowlist entries, add navigation labels, define its VitePress locale/search translations in `prepare.mjs`, and extend `verify-locales.mjs` to verify the new page set and search results. Do not silently fall back to Chinese content at an English route.

Internal navigation follows actual directories, with filenames as page labels and no empty-directory menus. `doc/local/plans/` is user-owned: do not rewrite or delete plans simply because implementation has changed. Keep temporary snapshots and capture artifacts under ignored `quality/artifacts/`, outside site sources and search.

## Build and verify

```sh
npm run build:local --prefix doc/site
python3 quality/tests/docs/verify_links.py doc/site/dist/local
npm run build:public --prefix doc/site
python3 quality/tests/docs/verify_links.py doc/site/dist/public
node doc/site/verify-locales.mjs
```

`dist/local/` includes private material; never publish it. `dist/public/` accepts only the explicit public allowlist. The public build rejects out-of-scope links, path escapes and symlinks, then audits input provenance and private-content markers. Hiding navigation is not content isolation.

`prepare.mjs` copies sources and rewrites links only in generated files. It renders local source snapshots as code pages when needed. `--refresh` deletes removed generated inputs; full builds regenerate all caches and output. Local and public modes have separate caches, configurations, indexes and artifacts.

`verify_links.py` checks all rendered local links, assets and anchors. `verify-locales.mjs` verifies paired pages, English/Chinese HTML languages, corresponding-page links, locale-specific sidebars, search-index isolation and real search results. These are generated-site checks, not assertions against copied prose.

## GitHub Pages

[English manual](https://txyy2023.github.io/log_print/) · [中文手册](https://txyy2023.github.io/log_print/zh/)

`.github/workflows/docs.yml` validates pushes to `dev` and `main`, plus manual runs. Only `main` deploys. It tests both the root path and the `/log_print/` deployment base, including locale checks. Reproduce the deployment build with:

```sh
DOCS_BASE=/log_print/ npm run build:public --prefix doc/site
python3 quality/tests/docs/verify_links.py doc/site/dist/public --base /log_print/
node doc/site/verify-locales.mjs doc/site/dist/public /log_print/
npm run preview:public --prefix doc/site
```

Preview at http://127.0.0.1:5174/log_print/. Rebuild without DOCS_BASE to restore root-path preview. Confirm actual workflow build and deployment results before reporting that the online site is updated.
