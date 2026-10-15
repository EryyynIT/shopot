//! Канал с ядром: [`CoreLink`], запросы, ответы, события.
//!
//! Трейт проектируется под будущий строчный протокол UDS
//! (`@<n> <команда>` → `@<n> ok|err`, события `imsg`/`msg`/`presence`/`sys`),
//! но не знает про сам сокет: реальный транспорт появится в
//! `uds.rs` одной заменой типа в [`crate::app`].

pub mod api;
pub mod mock;

use std::future::Future;
use std::sync::Arc;

use futures_core::Stream;

use crate::model::cid::Cid;
use crate::model::msg::MsgId;

/// Идентификатор запроса (тег `@<n>` в реальном протоколе).
pub type ReqId = u64;

/// Текущее время в мс — единая точка для ts у событий и сообщений.
pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Машинный формат списка контактов (затык 6 из LOCAL_PLANS.md):
/// `c <cid> <on|off> <b|-> <имя>`.
pub fn format_contact(c: &crate::model::store::Contact) -> String {
    format!(
        "c {} {} {} {}",
        c.cid,
        if c.online { "on" } else { "off" },
        if c.blocked { "b" } else { "-" },
        c.name
    )
}

/// Разбор машинного формата. Чужие строки — не паника, просто пропускаются.
pub fn parse_contacts(lines: &[String]) -> Vec<crate::model::store::Contact> {
    lines
        .iter()
        .filter_map(|line| {
            let mut parts = line.splitn(5, ' ');
            let tag = parts.next()?;
            if tag != "c" {
                return None;
            }
            let cid = parts.next()?.parse::<i64>().ok()?;
            let online = match parts.next()? {
                "on" => true,
                "off" => false,
                _ => return None,
            };
            let blocked = match parts.next()? {
                "-" => false,
                "b" => true,
                _ => return None,
            };
            let name = parts.next()?.to_owned();
            Some(crate::model::store::Contact {
                cid: Cid(cid),
                name,
                online,
                blocked,
            })
        })
        .collect()
}

/// Команда ядру. Состав повторяет команды TUI/REPL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Машинный список контактов: строки `c <cid> <on|off> <blocked> <имя>`.
    Contacts,
    /// Отправить сообщение контакту.
    SendMsg { cid: Cid, text: String },
    /// Добавить контакт по инвайт-коду `shopot1.…`.
    Add { code: String },
    /// Выдать свой инвайт-код.
    Invite,
    /// Заблокировать контакт.
    Block { cid: Cid },
    /// Разблокировать контакт.
    Unblock { cid: Cid },
}

/// Ответ на [`Request`]. Ошибки команды — данные, не паника: они едут
/// в Store как статус отправки и показываются пользователю.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Ok(Vec<String>),
    Err(String),
}

/// События ядра (поток, параллельный ответам).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkEvent {
    /// Входящее сообщение с msg_id (в реальном ядре — событие `imsg`).
    IMsg {
        cid: Cid,
        name: String,
        msg_id: MsgId,
        ts_ms: i64,
        text: String,
    },
    /// Presence контакта: ● онлайн / ○ офлайн.
    Presence { cid: Cid, online: bool },
    /// Служебная строка для ленты контакта.
    Sys { cid: Cid, text: String },
    /// Канал упал (для uds-транспорта; заглушка не присылает).
    Down,
}

/// Равноправная реализация транспорта ядра: заглушка сейчас, UDS позже.
///
/// [`CoreLink::events`] вызывается один раз (подписчик — iced),
/// [`CoreLink::execute`] — из асинхронного контекста iced (tokio).
pub trait CoreLink: Send + Sync + 'static {
    /// Поток событий ядра; забирает ручку линка (подписчик один).
    /// Требует tokio-контекста.
    fn events(self: Arc<Self>) -> impl Stream<Item = LinkEvent> + Send + 'static;

    /// Исполнить запрос и вернуть ответ. Будущее не заимствует линк:
    /// его можно уложить в Task и дождаться в update.
    fn execute(&self, request: Request) -> impl Future<Output = Reply> + Send + 'static;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::store::Contact;

    #[test]
    fn contact_line_roundtrip() {
        let contacts = [
            Contact {
                cid: Cid(1),
                name: "Аня".into(),
                online: true,
                blocked: false,
            },
            Contact {
                cid: Cid(2),
                name: "Борис О".into(),
                online: false,
                blocked: true,
            },
        ];
        let lines: Vec<String> = contacts.iter().map(format_contact).collect();
        assert_eq!(lines, ["c 1 on - Аня", "c 2 off b Борис О"]);

        let parsed = parse_contacts(&lines);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].cid, Cid(1));
        assert_eq!(parsed[0].name, "Аня");
        assert!(parsed[0].online);
        assert!(!parsed[0].blocked);
        assert_eq!(parsed[1].name, "Борис О", "имя может содержать пробелы");
        assert!(!parsed[1].online);
        assert!(parsed[1].blocked);
    }

    #[test]
    fn parse_skips_garbage_lines() {
        let lines = vec![
            "мусор".to_owned(),
            "c не_число on - X".to_owned(),
            "c 3 может off - X".to_owned(),
            "c 4 on - Вера".to_owned(),
        ];
        let parsed = parse_contacts(&lines);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "Вера");
    }
}
