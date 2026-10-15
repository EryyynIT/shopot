# shopot

[![CI](https://github.com/EryyynIT/shopot/actions/workflows/ci.yml/badge.svg)](https://github.com/EryyynIT/shopot/actions/workflows/ci.yml)

Визуальный десктоп-клиент для [shopot-core](https://github.com/Puxxalwl/shopot-core/) —
серверлесс P2P-движок обмена сообщениями со сквозным шифрованием и forward secrecy.

Собран на [iced](https://github.com/iced-rs/iced) с программным рендером
tiny-skia на Rust: тёмная палитра в духе TUI, один акцентный цвет, честные
статусы отправки, никаких фейковых квитанций.

![Скриншот](docs/screenshot.png)

## Статус

Ранняя разработка. Сейчас клиент общается со встроенной заглушкой ядра
(детерминированные демо-сцены); реальный транспорт к демону shopot-core
через Unix-сокет появится в позднем спринте. До продакшена ещё далеко.

Уже сделано:

- список контактов с точками presence и бейджами непрочитанных
- лента с разделителями дней, системными событиями и статусами отправки
- композер с авторостом и кнопкой отправки
- headless UI-тесты (`iced_test`) и юнит-тесты; CI гоняет fmt, clippy,
  тесты и release-сборку на каждый пуш

## Ядро

shopot — это только клиент; движок — шифрованный P2P-транспорт, сессии,
хранилище — живёт в [shopot-core](https://github.com/Puxxalwl/shopot-core)
от [puxxalwl](https://github.com/Puxxalwl).

## Сборка

Нужен стабильный Rust.

```sh
cargo run --release
```

## Лицензия

[MIT](LICENSE)
