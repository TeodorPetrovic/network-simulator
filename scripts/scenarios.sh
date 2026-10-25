#!/usr/bin/env bash
# REST and real-console traffic scenarios. Usage: scripts/scenarios.sh [file-pattern]
set -euo pipefail
cd "$(dirname "$0")/.."
if [[ ! -x target/debug/network-simulator ]]; then
  nice -n 10 cargo build -j2
fi
exec node --test --test-concurrency=1 --test-timeout=240000 ${1:-e2e/scenarios/*.test.mjs}
