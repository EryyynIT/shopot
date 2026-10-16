#!/usr/bin/env bash
# Приёмка перед коммитом: формат, линт, все тесты со скриншотами,
# e2e против обоих ядер. Падает на первой ошибке.
set -euo pipefail
cd "$(dirname "$0")/.."

# cpal (S7) линкуется с libasound: dev-файлы либо системные
# (alsa-lib-devel / libasound2-dev), либо локальная сборка в ~/.local
# (тарбол alsa-lib, ./configure --prefix=$HOME/.local) — без root.
if [ -f "$HOME/.local/lib/pkgconfig/alsa.pc" ]; then
    export PKG_CONFIG_PATH="$HOME/.local/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
fi

cargo fmt --check
cargo clippy --all-targets -- -D warnings
SHOPOT_SHOTS=1 cargo test
scripts/e2e.sh
