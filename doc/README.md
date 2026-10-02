# log_print documentation

English is the default public documentation language. Both versions cover the full public manual:

- [English manual](public/index.md) — canonical source under `doc/public/`.
- [简体中文手册](public/zh/index.md) — translations under `doc/public/zh/`.
- [Site maintenance](site/README.md) — local preview, locale maintenance, public builds and checks.

Internal development material and user-owned plans remain in the local `doc/local/` directory, with archive status at `doc/local/archive/`. These directories are not distributed with the public repository. Local workspaces that contain them can browse their actual directory hierarchy; their original language and URLs are preserved.

Start the local documentation service at http://127.0.0.1:5173/ using the maintenance guide. The public build includes only allowlisted public content. Only the `main` documentation workflow deploys it to GitHub Pages; local build output is not a deployment.
