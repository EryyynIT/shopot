#!/usr/bin/env bash
# e2e против обоих ядер: вендоренного core/ (API 1.x) и свежего клона
# ../shopot-core (API 2.x). Бинарники собираются, только если их нет.
set -euo pipefail
cd "$(dirname "$0")/.."

find_zig() {
    if [ -n "${ZIG:-}" ]; then echo "$ZIG"; return; fi
    if command -v zig >/dev/null 2>&1; then command -v zig; return; fi
    echo "$HOME/.local/opt/zig-0.17.0/zig"
}
ZIG_BIN="$(find_zig)"

run_against() {
    local root="$1"
    local bin="$root/zig/zig-out/bin/shopot-zig"
    if [ ! -x "$bin" ]; then
        echo "== сборка ядра: $root"
        (cd "$root/zig" && "$ZIG_BIN" build -Doptimize=ReleaseFast)
    fi
    echo "== e2e против $bin"
    SHOPOT_BIN="$bin" cargo test --test e2e_daemon
}

run_against core
run_against "${SHOPOT_CORE:-../shopot-core}"
