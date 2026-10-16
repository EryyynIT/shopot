//! Канал с ядром: [`CoreLink`], типизированные запросы, ответы, события.
//!
//! Контракт — бинарный протокол CORE-API ядра shopot (подмодуль `core/`
//! на 83ea91e, спецификация `core/zig/docs/CORE-API.md`; клиент совместим
//! с ядрами 1.x через зонд GET_VERSION): один REQUEST = один RESPONSE
//! с корреляцией по request_id, события рассылаются всем. Кодек кадров —
//! в [`api`], транспорт и раскладки payload — в [`uds`], демо-режим — в
//! [`mock`]. Строк на проводе нет: пользовательский текст появляется
//! только после [`LinkError::user_text`].

pub mod api;
pub mod mock;
pub mod uds;

use std::future::Future;
use std::hash::Hash;
use std::sync::Arc;

use futures_core::Stream;

use crate::model::cid::Cid;
use crate::model::msg::CoreMsgId;
use crate::model::store::Contact;

/// Текущее время в мс — единая точка для ts у событий и сообщений.
pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Потолок окна GET_HISTORY на стороне ядра; больше — TOO_BIG
/// (CORE-API.md §4.3). Пагинация GUI выше этого окна не поднимается.
pub const HISTORY_MAX_LIMIT: u32 = 200;

/// Команда ядру; одна команда = один метод API 1.0.0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Список контактов (LIST_CONTACTS).
    Contacts,
    /// Отправить сообщение (SEND_MESSAGE). Офлайн-контакт — не ошибка:
    /// ответ `queued` означает outbox ядра.
    SendMsg { cid: Cid, text: String },
    /// Добавить контакт по инвайт-коду (ACCEPT_INVITE).
    Add { code: String },
    /// Выдать свой инвайт-код (CREATE_INVITE). Пустой addr — код для одной
    /// локальной сети (ядро ждёт LAN-анонс); с адресом — ядро дозванивается
    /// сразу; relay — адрес трактуется как релей, peer IP не видит.
    Invite { relay: bool, addr: String },
    /// Заблокировать контакт (BLOCK_CONTACT).
    Block { cid: Cid },
    /// Разблокировать контакт (UNBLOCK_CONTACT).
    Unblock { cid: Cid },
    /// Версия API (GET_VERSION): readiness probe из CORE-API.md, первый
    /// запрос каждого соединения. От major зависят раскладки tail-зависимых
    /// методов (SEND_MESSAGE, GET_HISTORY).
    Version,
    /// История чата (GET_HISTORY): окно новейших записей, старые → новые.
    /// Лимит ядра — 200, 0 означает «дефолтные 50».
    History { cid: Cid, limit: u32 },
}

/// Ошибка канала с ядром (GUI_DEV.md §2: типизированные варианты).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkError {
    /// Демон не отвечает или соединение оборвалось.
    Disconnected,
    /// Ядро ответило машинным кодом статуса.
    Kernel(api::Status),
    /// Ответ не разобран — нарушение контракта со стороны ядра.
    BadFrame(&'static str),
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disconnected => write!(f, "нет связи с ядром"),
            Self::Kernel(status) => write!(f, "ядро: {}", status.name()),
            Self::BadFrame(what) => write!(f, "кадр не разобран: {what}"),
        }
    }
}

impl std::error::Error for LinkError {}

impl LinkError {
    /// Текст для пользователя: что случилось + что делать (TEXT_STYLE §3).
    /// Ядро шлёт только машинные коды — человеческий текст рождается здесь.
    pub fn user_text(&self) -> String {
        match self {
            Self::Disconnected => "Нет связи с ядром. Запустите shopot serve и подождите.".into(),
            Self::Kernel(api::Status::NoContact) => "Контакт не найден.".into(),
            Self::Kernel(api::Status::ContactBlocked) => {
                "Контакт заблокирован. Разблокируйте, чтобы писать.".into()
            }
            Self::Kernel(api::Status::BadInviteCode) => {
                "Неверный код. Скопируйте строку целиком — она начинается с shopot://i/.".into()
            }
            Self::Kernel(api::Status::TooManyInvites) => {
                "Активных приглашений уже четыре. Перезапустите shopot serve — реестр приглашений хранится в памяти демона.".into()
            }
            Self::Kernel(api::Status::TooBig) => {
                "Сообщение слишком длинное — лимит 4096 байт.".into()
            }
            Self::Kernel(api::Status::NoSession) => {
                "Нет защищённой сессии. Напишите, когда контакт будет онлайн.".into()
            }
            Self::Kernel(api::Status::Internal) => "Сбой ядра. Попробуйте позже.".into(),
            Self::Kernel(status) => {
                format!("Ошибка ядра ({}). Попробуйте позже.", status.name())
            }
            Self::BadFrame(_) => "Ядро ответило не по протоколу. Обновите shopot-core.".into(),
        }
    }
}

/// Запись GET_HISTORY: что ядро помнит о сообщении чата (CORE-API.md §4.3).
/// Сущности разметки ядро отдаёт в tail — клиент их пока не использует.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryMsg {
    pub mine: bool,
    /// Пир подтвердил ACK (bit1 flags). Осмысленно только для своих.
    pub delivered: bool,
    pub msg_id: CoreMsgId,
    pub ts_ms: i64,
    pub text: String,
}

/// Содержательный ответ при OK. Ошибки — через `Err(LinkError)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OkReply {
    /// LIST_CONTACTS: контакты в порядке ядра.
    Contacts(Vec<Contact>),
    /// SEND_MESSAGE: msg_id ядра + queued (true = ушло в outbox).
    Sent { msg_id: CoreMsgId, queued: bool },
    /// ACCEPT_INVITE: cid добавленного контакта.
    Added { cid: Cid },
    /// CREATE_INVITE: односторонний инвайт-код.
    Code(String),
    /// GET_VERSION: версия API демона, semver.
    Version { major: u16, minor: u16, patch: u16 },
    /// GET_HISTORY: всего записей у ядра + окно новейших (старые → новые).
    History {
        total: u32,
        records: Vec<HistoryMsg>,
    },
    /// У команды нет содержательного ответа (block/unblock).
    Done,
}

/// Ответ на [`Request`]. Ошибки команды — данные, не паника: они едут
/// в Store как статус отправки и показываются пользователю.
pub type Reply = Result<OkReply, LinkError>;

/// События ядра (рассылка всем клиентам, CORE-API.md §5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkEvent {
    /// Входящее сообщение.
    MessageReceived {
        cid: Cid,
        name: String,
        msg_id: CoreMsgId,
        ts_ms: i64,
        text: String,
    },
    /// ACK: сообщение доставлено на устройство пира.
    MessageDelivered { cid: Cid, msg_id: CoreMsgId },
    /// Presence контакта; сессия = живое соединение.
    Presence {
        cid: Cid,
        online: bool,
        last_seen_ms: i64,
    },
    /// Держатель нашего инвайта подключился.
    ContactJoined { cid: Cid, name: String },
    /// Профиль контакта обновился (имя; avatar_id — с S8).
    ContactProfile {
        cid: Cid,
        name: String,
        avatar_id: CoreMsgId,
    },
    /// Пир заблокировал нас.
    BlockedByPeer { cid: Cid },
    /// Соединение с демоном потеряно (транспорт; заглушка не присылает).
    Down,
    /// Соединение восстановлено — обновить контакты (ресинк).
    Resync,
}

/// Равноправная реализация транспорта ядра: заглушка сейчас, UDS рядом.
///
/// [`CoreLink::events`] вызывается один раз (подписчик — iced),
/// [`CoreLink::execute`] — из асинхронного контекста iced (tokio).
/// `Hash` — идентичность подписки iced (`Subscription::run_with`).
pub trait CoreLink: Send + Sync + Hash + 'static {
    /// Поток событий ядра; забирает ручку линка (подписчик один).
    /// Требует tokio-контекста.
    fn events(self: Arc<Self>) -> impl Stream<Item = LinkEvent> + Send + 'static;

    /// Исполнить запрос и вернуть ответ. Будущее не заимствует линк:
    /// его можно уложить в Task и дождаться в update.
    fn execute(&self, request: Request) -> impl Future<Output = Reply> + Send + 'static;

    /// Заглушка это или реальное ядро — для подписей в UI.
    fn is_stub(&self) -> bool {
        false
    }
}

/// Мост tokio mpsc → futures Stream (без tokio-stream: poll_recv есть
/// публично). Общий для mock и uds.
pub(crate) struct RecvStream<T>(tokio::sync::mpsc::Receiver<T>);

impl<T> Stream for RecvStream<T> {
    type Item = T;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<T>> {
        // tokio::sync::mpsc::Receiver::poll_recv принимает &mut self.
        self.get_mut().0.poll_recv(cx)
    }
}

#[cfg(test)]
pub(crate) async fn take_events(
    mut stream: impl Stream<Item = LinkEvent> + Unpin,
    n: usize,
) -> std::collections::VecDeque<LinkEvent> {
    use futures_util::StreamExt;
    use std::collections::VecDeque;
    let mut out = VecDeque::with_capacity(n);
    for _ in 0..n {
        if let Some(event) = stream.next().await {
            out.push_back(event);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Совет в тексте ошибки обязан быть выполнимым в GUI (TEXT_STYLE §3):
    /// отзыва инвайтов в API 1.0.0 нет, перезапуск демона — есть.
    #[test]
    fn too_many_invites_advises_restart_not_revoke() {
        let err = LinkError::Kernel(api::Status::TooManyInvites);
        let text = err.user_text();
        assert!(
            text.contains("Перезапустите"),
            "совет должен быть выполнимым: {text}"
        );
        assert!(!text.contains("Отзовите"), "отзыва в API нет: {text}");
    }
}
