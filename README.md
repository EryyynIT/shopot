# shopot

[![CI](https://github.com/EryyynIT/shopot/actions/workflows/ci.yml/badge.svg)](https://github.com/EryyynIT/shopot/actions/workflows/ci.yml)

Desktop GUI client for [shopot-core](https://github.com/Puxxalwl/shopot-core/) —
a serverless P2P messaging engine with end-to-end encryption and forward secrecy.

Built with [iced](https://github.com/iced-rs/iced) on the tiny-skia software
renderer in Rust: dark TUI-derived palette, one accent color, honest send
statuses, no fake delivery receipts.

[README на русском](README.ru.md)

![Screenshot](docs/screenshot.png)

## Status

Early development. The client talks to a real `shopot serve` daemon over its
binary frame API 1.0.0 on a Unix domain socket (`$HOME/.shopot/api.sock`);
`--mock` runs a built-in demo scene instead. Nothing here is production-ready
yet.

Done so far:

- typed core link: binary frame codec, request correlation, events, reconnect
  with resync; an e2e test runs invite → message → delivery ACK against two
  real daemons (gated by `SHOPOT_BIN`)
- contacts list with presence dots and unread badges
- message feed with day separators, system events and honest send statuses
  (sending → sent / queued / delivered / failed)
- auto-growing composer with a send button
- headless UI tests (`iced_test`) plus unit tests; CI runs fmt, clippy, tests
  and a release build on every push

## Build

Requires a stable Rust toolchain.

```sh
cargo run --release
```

## The core

shopot itself is the client; the engine — encrypted P2P transport, sessions,
storage — lives in [shopot-core](https://github.com/Puxxalwl/shopot-core) by
[puxxalwl](https://github.com/Puxxalwl). The pinned API contract is vendored
as the `core` git submodule (binary protocol spec:
`core/zig/docs/CORE-API.md`); build the daemon with Zig 0.17.0:

```sh
cd core/zig && zig build -Doptimize=ReleaseSafe
```

## License

[MIT](LICENSE)
