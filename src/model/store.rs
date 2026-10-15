//! Store: проекция состояния ядра. cid — единственный ключ.

use std::collections::HashMap;

use crate::model::cid::Cid;
use crate::model::feed::{self, FeedItem};
use crate::model::msg::{Msg, MsgId, SendState};

/// Контакт.
#[derive(Clone, Debug)]
pub struct Contact {
    pub cid: Cid,
    pub name: String,
    pub online: bool,
    pub blocked: bool,
}

/// Состояние защищённой сессии с контактом. Пока ядро присылает сессию
/// только текстом sys-строк — разбираем их подписи (I-15); в S5 заменить
/// на структурированное событие (Q15 бэкенду).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Session {
    #[default]
    Unknown,
    Up,
    Down,
}

/// Лента чата: сообщения, непрочитанные, черновик, сессия
/// (в памяти, не на диске).
#[derive(Clone, Debug, Default)]
pub struct Thread {
    pub msgs: Vec<Msg>,
    pub unread: u32,
    pub draft: String,
    pub session: Session,
}

/// Проекция ядра для UI. Пересобирается из contacts + событий.
#[derive(Debug, Default)]
pub struct Store {
    contacts: HashMap<Cid, Contact>,
    order: Vec<Cid>,
    threads: HashMap<Cid, Thread>,
    selected: Option<Cid>,
    next_msg_id: u64,
}

impl Store {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ресинк контактов из машинного ответа ядра. Ленты не трогает.
    pub fn replace_contacts(&mut self, contacts: Vec<Contact>) {
        self.order = contacts.iter().map(|c| c.cid).collect();
        for contact in contacts {
            self.threads.entry(contact.cid).or_default();
            self.contacts.insert(contact.cid, contact);
        }
        self.order.retain(|cid| self.contacts.contains_key(cid));
    }

    /// Контакты в порядке ядра.
    pub fn contacts(&self) -> impl Iterator<Item = &Contact> + '_ {
        self.order.iter().filter_map(|cid| self.contacts.get(cid))
    }

    pub fn contact(&self, cid: Cid) -> Option<&Contact> {
        self.contacts.get(&cid)
    }

    pub fn online_count(&self) -> usize {
        self.contacts.values().filter(|c| c.online).count()
    }

    pub fn selected(&self) -> Option<Cid> {
        self.selected
    }

    /// Открыть чат: сбрасывает непрочитанные выбранного.
    pub fn select(&mut self, cid: Option<Cid>) {
        self.selected = cid;
        if let Some(cid) = cid {
            if let Some(thread) = self.threads.get_mut(&cid) {
                thread.unread = 0;
            }
        }
    }

    /// Входящее сообщение. От неизвестного — контакт создаётся (имя из события).
    pub fn push_incoming(&mut self, cid: Cid, name: &str, msg_id: MsgId, ts_ms: i64, text: String) {
        let fresh = !self.contacts.contains_key(&cid);
        if fresh {
            self.order.push(cid);
            self.threads.entry(cid).or_default();
            self.contacts.insert(
                cid,
                Contact {
                    cid,
                    name: name.to_owned(),
                    online: true,
                    blocked: false,
                },
            );
        }
        let unread = if self.selected == Some(cid) { 0 } else { 1 };
        let thread = self.threads.entry(cid).or_default();
        thread.unread += unread;
        thread.msgs.push(Msg::incoming(msg_id, cid, ts_ms, text));
    }

    /// Моё сообщение: сразу `Sending`, идентификатор выдаёт Store.
    pub fn push_outgoing(&mut self, cid: Cid, ts_ms: i64, text: String) -> MsgId {
        self.next_msg_id += 1;
        let id = MsgId(self.next_msg_id);
        self.threads
            .entry(cid)
            .or_default()
            .msgs
            .push(Msg::outgoing(id, ts_ms, text));
        id
    }

    pub fn push_sys(&mut self, cid: Cid, ts_ms: i64, text: String) {
        let thread = self.threads.entry(cid).or_default();
        // Хак до Q15: событие сессии приходит строкой. Подписи — из
        // демо-сцены/ядра; ложные срабатывания на обычных сообщениях
        // исключены: push_sys вызывается только для sys-событий.
        if text.contains("сессия установлена") {
            thread.session = Session::Up;
        } else if text.contains("сессия разорвана") || text.contains("сессия закрыта")
        {
            thread.session = Session::Down;
        }
        thread.msgs.push(Msg::sys(ts_ms, text));
    }

    /// Состояние сессии с контактом — для подзаголовка шапки чата.
    pub fn session(&self, cid: Cid) -> Session {
        self.threads
            .get(&cid)
            .map(|t| t.session)
            .unwrap_or(Session::Unknown)
    }

    pub fn mark_sent(&mut self, cid: Cid, msg_id: MsgId) {
        self.set_send_state(cid, msg_id, SendState::Sent);
    }

    pub fn mark_failed(&mut self, cid: Cid, msg_id: MsgId, reason: String) {
        self.set_send_state(cid, msg_id, SendState::Failed(reason));
    }

    fn set_send_state(&mut self, cid: Cid, msg_id: MsgId, send: SendState) {
        let Some(thread) = self.threads.get_mut(&cid) else {
            return;
        };
        for msg in thread.msgs.iter_mut().rev() {
            if msg.id == msg_id {
                msg.send = send;
                return;
            }
        }
    }

    pub fn set_presence(&mut self, cid: Cid, online: bool) {
        if let Some(contact) = self.contacts.get_mut(&cid) {
            contact.online = online;
        }
    }

    pub fn thread(&self, cid: Cid) -> Option<&Thread> {
        self.threads.get(&cid)
    }

    pub fn unread_total(&self) -> u32 {
        self.threads.values().map(|t| t.unread).sum()
    }

    pub fn draft(&self, cid: Cid) -> &str {
        self.threads
            .get(&cid)
            .map(|t| t.draft.as_str())
            .unwrap_or("")
    }

    pub fn set_draft(&mut self, cid: Cid, draft: String) {
        self.threads.entry(cid).or_default().draft = draft;
    }

    /// Лента чата, сгруппированная для отображения; имена авторов
    /// подставлены из контактов.
    pub fn feed(&self, cid: Cid, day_of: impl Fn(i64) -> i64) -> Vec<FeedItem> {
        let Some(thread) = self.threads.get(&cid) else {
            return Vec::new();
        };
        let mut items = feed::group(&thread.msgs, day_of);
        for item in items.iter_mut() {
            let FeedItem::Group { author, name, .. } = item else {
                continue;
            };
            let Some(author) = *author else { continue };
            *name = self
                .contacts
                .get(&author)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| author.to_string());
        }
        items
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sys_text_tracks_session_state() {
        let mut store = Store::default();
        store.replace_contacts(vec![contact(1, "Аня", true)]);
        assert_eq!(store.session(Cid(1)), Session::Unknown);
        store.push_sys(Cid(1), 1_000, "сессия установлена".into());
        assert_eq!(store.session(Cid(1)), Session::Up);
        store.push_sys(Cid(1), 2_000, "сессия разорвана".into());
        assert_eq!(store.session(Cid(1)), Session::Down);
    }

    const DAY: i64 = 24 * 60 * 60 * 1000;

    fn contact(cid: i64, name: &str, online: bool) -> Contact {
        Contact {
            cid: Cid(cid),
            name: name.to_owned(),
            online,
            blocked: false,
        }
    }

    #[test]
    fn replace_contacts_keeps_kernel_order_and_threads() {
        let mut store = Store::new();
        store.replace_contacts(vec![contact(1, "Аня", true), contact(2, "Борис", false)]);
        let names: Vec<&str> = store.contacts().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Аня", "Борис"]);
        assert_eq!(store.online_count(), 1);

        store.push_incoming(Cid(2), "Борис", MsgId(1), 1000, "привет".into());
        store.replace_contacts(vec![contact(1, "Аня", false), contact(2, "Борис", true)]);
        assert_eq!(
            store.thread(Cid(2)).unwrap().unread,
            1,
            "ресинк не трогает ленту"
        );
    }

    #[test]
    fn incoming_to_selected_chat_is_read() {
        let mut store = Store::new();
        store.replace_contacts(vec![contact(1, "Аня", true)]);
        store.select(Some(Cid(1)));
        store.push_incoming(Cid(1), "Аня", MsgId(1), 1000, "привет".into());
        assert_eq!(store.unread_total(), 0);

        store.select(None);
        store.push_incoming(Cid(1), "Аня", MsgId(2), 2000, "как дела".into());
        assert_eq!(store.unread_total(), 1);
        store.select(Some(Cid(1)));
        assert_eq!(
            store.unread_total(),
            0,
            "открытие чата читает непрочитанные"
        );
    }

    #[test]
    fn outgoing_send_states() {
        let mut store = Store::new();
        store.replace_contacts(vec![contact(1, "Аня", true)]);
        let id = store.push_outgoing(Cid(1), 1000, "привет".into());
        let msgs = &store.thread(Cid(1)).unwrap().msgs;
        assert_eq!(msgs[0].send, SendState::Sending);

        store.mark_failed(Cid(1), id, "нет связи".into());
        assert_eq!(
            store.thread(Cid(1)).unwrap().msgs[0].send,
            SendState::Failed("нет связи".into())
        );
        store.mark_sent(Cid(1), id);
        assert_eq!(store.thread(Cid(1)).unwrap().msgs[0].send, SendState::Sent);
    }

    #[test]
    fn drafts_survive_chat_switch() {
        let mut store = Store::new();
        store.replace_contacts(vec![contact(1, "Аня", true), contact(2, "Борис", true)]);
        store.set_draft(Cid(1), "черновик Ане".into());
        assert_eq!(store.draft(Cid(1)), "черновик Ане");
        assert_eq!(store.draft(Cid(2)), "");
        store.set_draft(Cid(1), "".into());
        assert_eq!(store.draft(Cid(1)), "");
    }

    #[test]
    fn feed_groups_by_author_gap_and_day() {
        let mut store = Store::new();
        store.replace_contacts(vec![contact(1, "Аня", true)]);
        // Вчерашний день, Аня, два подряд; потом мой ответ через 6 минут
        // (новая группа), потом новое сутки — новый разделитель.
        let yesterday = DAY;
        store.push_incoming(Cid(1), "Аня", MsgId(1), yesterday, "привет".into());
        store.push_incoming(Cid(1), "Аня", MsgId(2), yesterday + 1000, "как дела".into());
        store.push_outgoing(Cid(1), yesterday + 6 * 60 * 1000, "норм".into());
        store.push_incoming(Cid(1), "Аня", MsgId(3), yesterday + DAY, "ок".into());

        let items = store.feed(Cid(1), |ts| ts / DAY);
        assert!(matches!(items[0], FeedItem::DateSep { .. }));
        let FeedItem::Group {
            mine, bodies, name, ..
        } = &items[1]
        else {
            panic!("группа")
        };
        assert!(!*mine);
        assert_eq!(bodies.len(), 2);
        assert_eq!(name, "Аня", "имя автора подставлено из контакта");
        let FeedItem::Group { mine, bodies, .. } = &items[2] else {
            panic!("группа")
        };
        assert!(mine, "смена автора рвёт группу");
        assert_eq!(bodies.len(), 1);
        assert!(matches!(items[3], FeedItem::DateSep { .. }));
        assert_eq!(items.len(), 5);
    }
}
