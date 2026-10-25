#!/usr/bin/env bash
# Browser end-to-end suite. Usage: scripts/e2e.sh [file-pattern]   (E2E_BROWSER=firefox to switch)
set -euo pipefail
cd "$(dirname "$0")/.."
exec node --test --test-concurrency=1 --test-timeout=120000 ${1:-e2e/*.test.mjs}
