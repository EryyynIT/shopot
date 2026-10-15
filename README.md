# shopot

[![CI](https://github.com/EryyynIT/shopot/actions/workflows/ci.yml/badge.svg)](https://github.com/EryyynIT/shopot/actions/workflows/ci.yml)

Desktop GUI client for [shopot-core](https://github.com/Puxxalwl/shopot-core/) —
a serverless P2P messaging engine with end-to-end encryption and forward secrecy.

Built with [iced](https://github.com/iced-rs/iced) on the tiny-skia software
renderer in Rust: dark TUI-derived palette, one accent color, honest send
statuses, no fake delivery receipts.

![Screenshot](docs/screenshot.png)

## Status

Early development. The client currently talks to a built-in mock of the core
link (deterministic demo scenes); the real shopot-core daemon transport over
a Unix domain socket lands in a later sprint. Nothing here is production-ready
yet.

Done so far:

- contacts list with presence dots and unread badges
- message feed with day separators, system events and per-group send status
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
[puxxalwl](https://github.com/Puxxalwl).

## License

[MIT](LICENSE)
