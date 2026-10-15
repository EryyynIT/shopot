# shopot

[![CI](https://github.com/EryyynIT/shopot/actions/workflows/ci.yml/badge.svg)](https://github.com/EryyynIT/shopot/actions/workflows/ci.yml)

Визуальный десктоп-клиент для [shopot-core](https://github.com/Puxxalwl/shopot-core/) —
серверлесс P2P-движок обмена сообщениями со сквозным шифрованием и forward secrecy.

Собран на [iced](https://github.com/iced-rs/iced) с программным рендером
tiny-skia на Rust: тёмная палитра в духе TUI, один акцентный цвет, честные
статусы отправки, никаких фейковых квитанций.

![Скриншот](docs/screenshot.png)

## Статус

Ранняя разработка. Клиент работает с реальным демоном `shopot serve` по
бинарному кадровому API 1.0.0 через Unix-сокет (`$HOME/.shopot/api.sock`);
`--mock` включает демо-сцену без ядра. До продакшена ещё далеко.

Уже сделано:

- типизированный канал ядра: кодек бинарных кадров, корреляция ответов,
  события, реконнект с ресинком; e2e-тест гоняет инвайт → сообщение → ACK
  доставки против двух реальных демонов (гейт по `SHOPOT_BIN`)
- список контактов с точками presence и бейджами непрочитанных
- лента с разделителями дней, системными событиями и честными статусами
  (отправляю → отправлено / отправится позже / доставлено / не ушло)
- композер с авторостом и кнопкой отправки
- headless UI-тесты (`iced_test`) и юнит-тесты; CI гоняет fmt, clippy,
  тесты и release-сборку на каждый пуш

## Ядро

shopot — это только клиент; движок — шифрованный P2P-транспорт, сессии,
хранилище — живёт в [shopot-core](https://github.com/Puxxalwl/shopot-core)
от [puxxalwl](https://github.com/Puxxalwl). Зафиксированный контракт API —
подмодуль `core` (спецификация бинарного протокола:
`core/zig/docs/CORE-API.md`); демон собирается Zig 0.17.0:

```sh
cd core/zig && zig build -Doptimize=ReleaseSafe
```

## Сборка

Нужен стабильный Rust.

```sh
cargo run --release
```

## Лицензия

[MIT](LICENSE)
