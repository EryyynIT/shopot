#!/bin/sh
# shopot одним архивом: первый запуск создаёт личность, дальше —
# демон в фоне и окно клиента. Linux x86_64.
set -e
cd "$(dirname "$0")"

# init на повторном запуске не нужен, но и не вредит: демону нужна личность.
./shopot-zig init || true
./shopot-zig serve &
daemon_pid=$!

# UdsLink переподключается сам, но сокет пусть успеет открыться.
sleep 0.3
./shopot-gui || true

kill "$daemon_pid" 2>/dev/null || true
