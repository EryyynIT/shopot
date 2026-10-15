//! Сообщение ленты: направление, время, статус отправки.

use std::fmt;

use crate::model::cid::Cid;

/// Локальный номер строки ленты — ключ UI (привязка ответа к пузырю).
/// Не путать с [`CoreMsgId`]: настоящий msg_id ядра живёт в `Msg::core_id`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct MsgId(pub u64);

/// msg_id ядра: 16 случайных байт, идентифицируют сообщение глобально
/// (CORE-API.md §1). У моих сообщений появляется после ответа SEND_MESSAGE,
/// у чужих — приходит в событии MESSAGE_RECEIVED.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct CoreMsgId(pub [u8; 16]);

impl fmt::Display for CoreMsgId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Display for MsgId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

/// Автор строки ленты.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dir {
    /// Моё сообщение.
    Out,
    /// Сообщение собеседника.
    In(Cid),
    /// Служебная строка ядра (sys).
    Sys,
}

/// Честный статус отправки — жизненный цикл моего сообщения
/// по модели доставки ядра (CORE-API.md §6).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SendState {
    Sending,
    Sent,
    /// Ядро положило сообщение в outbox: уйдёт, когда контакт появится.
    Queued,
    /// Пир подтвердил ACK — сообщение на его устройстве.
    Delivered,
    /// Не ушло + причина для пользователя.
    Failed(String),
}

/// Строка ленты.
#[derive(Clone, Debug)]
pub struct Msg {
    pub id: MsgId,
    pub dir: Dir,
    pub ts_ms: i64,
    pub body: String,
    pub send: SendState,
    /// msg_id ядра, если известен; по нему приходит MESSAGE_DELIVERED.
    pub core_id: Option<CoreMsgId>,
}

impl Msg {
    pub fn incoming(
        id: MsgId,
        cid: Cid,
        ts_ms: i64,
        body: String,
        core_id: Option<CoreMsgId>,
    ) -> Self {
        Self {
            id,
            dir: Dir::In(cid),
            ts_ms,
            body,
            send: SendState::Sent,
            core_id,
        }
    }

    pub fn outgoing(id: MsgId, ts_ms: i64, body: String) -> Self {
        Self {
            id,
            dir: Dir::Out,
            ts_ms,
            body,
            send: SendState::Sending,
            core_id: None,
        }
    }

    pub fn sys(ts_ms: i64, body: String) -> Self {
        Self {
            id: MsgId(0),
            dir: Dir::Sys,
            ts_ms,
            body,
            send: SendState::Sent,
            core_id: None,
        }
    }

    /// Автор группы: мои и sys всегда свои группы; чужие — по cid.
    pub fn is_same_group_as(&self, other: &Msg) -> bool {
        match (&self.dir, &other.dir) {
            (Dir::In(a), Dir::In(b)) => a == b,
            (Dir::Out, Dir::Out) => true,
            (Dir::Sys, Dir::Sys) => true,
            _ => false,
        }
    }
}
