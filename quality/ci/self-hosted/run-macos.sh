#!/usr/bin/env bash
# Preserve the caller/runner PATH; do not start a login shell.
set -euo pipefail
ci_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
if [[ "$(uname -s)" != Darwin ]]; then
  echo 'This entry point requires macOS; use cargo run --locked -p log-print-quality -- on other platforms.' >&2
  exit 2
fi
if [[ "${LOG_PRINT_SELF_HOSTED_ENABLED:-false}" != true ]]; then
  echo 'Set LOG_PRINT_SELF_HOSTED_ENABLED=true to run the local formal validation suite.' >&2
  exit 2
fi
command -v cargo
cargo --version
command -v node
node --version
command -v npm
npm --version
cd "$ci_root"
exec cargo run --locked -p log-print-quality -- "$@"
