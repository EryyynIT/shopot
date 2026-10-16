#!/usr/bin/env bash
# Приёмка перед коммитом: формат, линт, все тесты со скриншотами,
# e2e против обоих ядер. Падает на первой ошибке.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo fmt --check
cargo clippy --all-targets -- -D warnings
SHOPOT_SHOTS=1 cargo test
scripts/e2e.sh
