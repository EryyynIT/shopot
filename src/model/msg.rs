//! Сообщение ленты: направление, время, статус отправки.

use std::fmt;

use crate::model::cid::Cid;

/// Идентификатор сообщения. Пока ядро не отдаёт `imsg`, клиент
/// генерирует свои: дедуп и реакции появятся вместе с `imsg`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct MsgId(pub u64);

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

/// Честный статус отправки: три состояния, пока нет outbox ядра.
/// Никаких «доставлено/прочитано» без квитанции от ядра.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SendState {
    Sending,
    Sent,
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
}

impl Msg {
    pub fn incoming(id: MsgId, cid: Cid, ts_ms: i64, body: String) -> Self {
        Self {
            id,
            dir: Dir::In(cid),
            ts_ms,
            body,
            send: SendState::Sent,
        }
    }

    pub fn outgoing(id: MsgId, ts_ms: i64, body: String) -> Self {
        Self {
            id,
            dir: Dir::Out,
            ts_ms,
            body,
            send: SendState::Sending,
        }
    }

    pub fn sys(ts_ms: i64, body: String) -> Self {
        Self {
            id: MsgId(0),
            dir: Dir::Sys,
            ts_ms,
            body,
            send: SendState::Sent,
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
