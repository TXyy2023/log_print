#!/usr/bin/env bash
# Preserve the caller/runner PATH; do not start a login shell.
set -euo pipefail
ci_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
if [[ "$(uname -s)" != Darwin ]]; then
  echo 'This entry point requires macOS; use python3 tests/run.py on other platforms.' >&2
  exit 2
fi
if [[ "${LOG_PRINT_SELF_HOSTED_ENABLED:-false}" != true ]]; then
  echo 'Set LOG_PRINT_SELF_HOSTED_ENABLED=true to run the local formal validation suite.' >&2
  exit 2
fi
ci_python="${LOG_PRINT_CI_PYTHON:-python3}"
command -v "$ci_python"
"$ci_python" --version
command -v cargo
cargo --version
cd "$ci_root"
exec "$ci_python" tests/run.py "$@"
