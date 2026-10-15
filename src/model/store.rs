//! Store: проекция состояния ядра. cid — единственный ключ.

use std::collections::HashMap;

use crate::model::cid::Cid;
use crate::model::feed::{self, FeedItem};
use crate::model::msg::{CoreMsgId, Msg, MsgId, SendState};

/// Контакт.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Contact {
    pub cid: Cid,
    pub name: String,
    pub online: bool,
    pub blocked: bool,
}

/// Состояние защищённой сессии с контактом. Ядро отдельного события сессии
/// не шлёт: сессия = живое соединение, поэтому её определяет PRESENCE.
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

    /// Входящее сообщение от известного/нового контакта; имя — из события.
    /// Новые сообщения ядра идут через [`Self::push_incoming_core`].
    pub fn push_incoming(&mut self, cid: Cid, name: &str, msg_id: MsgId, ts_ms: i64, text: String) {
        self.push_incoming_impl(cid, name, None, msg_id, ts_ms, text);
    }

    /// Входящее сообщение с настоящим msg_id ядра (событие MESSAGE_RECEIVED).
    pub fn push_incoming_core(
        &mut self,
        cid: Cid,
        name: &str,
        core_id: CoreMsgId,
        ts_ms: i64,
        text: String,
    ) {
        let local = self.next_local_id();
        self.push_incoming_impl(cid, name, Some(core_id), local, ts_ms, text);
    }

    fn next_local_id(&mut self) -> MsgId {
        self.next_msg_id += 1;
        MsgId(self.next_msg_id)
    }

    fn push_incoming_impl(
        &mut self,
        cid: Cid,
        name: &str,
        core_id: Option<CoreMsgId>,
        msg_id: MsgId,
        ts_ms: i64,
        text: String,
    ) {
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
        thread
            .msgs
            .push(Msg::incoming(msg_id, cid, ts_ms, text, core_id));
    }

    /// Моё сообщение: сразу `Sending`, идентификатор выдаёт Store.
    pub fn push_outgoing(&mut self, cid: Cid, ts_ms: i64, text: String) -> MsgId {
        let id = self.next_local_id();
        self.threads
            .entry(cid)
            .or_default()
            .msgs
            .push(Msg::outgoing(id, ts_ms, text));
        id
    }

    /// Служебная строка ядра в ленте (контакт подключился, заблокировал…).
    pub fn push_sys(&mut self, cid: Cid, ts_ms: i64, text: String) {
        self.threads
            .entry(cid)
            .or_default()
            .msgs
            .push(Msg::sys(ts_ms, text));
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

    /// Ответ SEND_MESSAGE: сообщение ушло в сессию, ядровый msg_id привязан.
    pub fn mark_sent_core(&mut self, cid: Cid, msg_id: MsgId, core_id: CoreMsgId) {
        if let Some(msg) = self.msg_mut(cid, msg_id) {
            msg.core_id = Some(core_id);
            msg.send = SendState::Sent;
        }
    }

    /// Ответ SEND_MESSAGE с queued=1: сообщение в outbox, уедет при онлайне.
    pub fn mark_queued(&mut self, cid: Cid, msg_id: MsgId) {
        self.set_send_state(cid, msg_id, SendState::Queued);
    }

    pub fn mark_failed(&mut self, cid: Cid, msg_id: MsgId, reason: String) {
        self.set_send_state(cid, msg_id, SendState::Failed(reason));
    }

    /// Событие MESSAGE_DELIVERED: пир подтвердил получение по msg_id ядра.
    pub fn mark_delivered(&mut self, cid: Cid, core_id: CoreMsgId) {
        let Some(thread) = self.threads.get_mut(&cid) else {
            return;
        };
        for msg in thread.msgs.iter_mut().rev() {
            if msg.core_id == Some(core_id) {
                msg.send = SendState::Delivered;
                return;
            }
        }
    }

    fn msg_mut(&mut self, cid: Cid, msg_id: MsgId) -> Option<&mut Msg> {
        self.threads
            .get_mut(&cid)?
            .msgs
            .iter_mut()
            .rev()
            .find(|m| m.id == msg_id)
    }

    fn set_send_state(&mut self, cid: Cid, msg_id: MsgId, send: SendState) {
        if let Some(msg) = self.msg_mut(cid, msg_id) {
            msg.send = send;
        }
    }

    /// PRESENCE ядра: онлайн-флаг и производная от него сессия.
    pub fn set_presence(&mut self, cid: Cid, online: bool) {
        if let Some(contact) = self.contacts.get_mut(&cid) {
            contact.online = online;
        }
        if let Some(thread) = self.threads.get_mut(&cid) {
            thread.session = if online { Session::Up } else { Session::Down };
        }
    }

    /// Контакт появился (CONTACT_JOINED): создаём, не трогая существующего.
    pub fn ensure_contact(&mut self, cid: Cid, name: &str) -> bool {
        if self.contacts.contains_key(&cid) {
            return false;
        }
        self.order.push(cid);
        self.threads.entry(cid).or_default();
        self.contacts.insert(
            cid,
            Contact {
                cid,
                name: name.to_owned(),
                online: false,
                blocked: false,
            },
        );
        true
    }

    /// CONTACT_PROFILE: имя контакта обновилось.
    pub fn rename_contact(&mut self, cid: Cid, name: &str) {
        if let Some(contact) = self.contacts.get_mut(&cid) {
            contact.name = name.to_owned();
        }
    }

    /// Ответ BLOCK_CONTACT/UNBLOCK_CONTACT: флаг блокировки контакта.
    pub fn set_blocked(&mut self, cid: Cid, blocked: bool) {
        if let Some(contact) = self.contacts.get_mut(&cid) {
            contact.blocked = blocked;
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
    fn presence_drives_session_state() {
        let mut store = Store::default();
        store.replace_contacts(vec![contact(1, "Аня", false)]);
        assert_eq!(store.session(Cid(1)), Session::Unknown);
        store.set_presence(Cid(1), true);
        assert_eq!(store.session(Cid(1)), Session::Up);
        store.set_presence(Cid(1), false);
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

    /// Жизненный цикл по модели доставки ядра (CORE-API.md §6):
    /// отправка → queued (контакт офлайн) → sent (ушло) → delivered (ACK).
    #[test]
    fn delivery_lifecycle_queued_sent_delivered() {
        let mut store = Store::new();
        store.replace_contacts(vec![contact(1, "Аня", false)]);

        let id = store.push_outgoing(Cid(1), 1000, "привет".into());
        store.mark_queued(Cid(1), id);
        assert_eq!(
            store.thread(Cid(1)).unwrap().msgs[0].send,
            SendState::Queued,
            "офлайн-отправка — это outbox, не ошибка"
        );

        let core = CoreMsgId([0xAA; 16]);
        store.mark_sent_core(Cid(1), id, core);
        assert_eq!(store.thread(Cid(1)).unwrap().msgs[0].send, SendState::Sent);
        assert_eq!(store.thread(Cid(1)).unwrap().msgs[0].core_id, Some(core));

        store.mark_delivered(Cid(1), core);
        assert_eq!(
            store.thread(Cid(1)).unwrap().msgs[0].send,
            SendState::Delivered
        );
    }

    #[test]
    fn delivered_matches_by_core_id_of_mine_and_theirs() {
        let mut store = Store::new();
        store.replace_contacts(vec![contact(1, "Аня", true)]);
        let mine = CoreMsgId([1; 16]);
        let theirs = CoreMsgId([2; 16]);

        let id = store.push_outgoing(Cid(1), 1000, "моё".into());
        store.mark_sent_core(Cid(1), id, mine);
        store.push_incoming_core(Cid(1), "Аня", theirs, 1100, "чужое".into());

        store.mark_delivered(Cid(1), theirs);
        assert_eq!(
            store.thread(Cid(1)).unwrap().msgs[0].send,
            SendState::Sent,
            "чужой msg_id не трогает моё"
        );
        store.mark_delivered(Cid(1), mine);
        assert_eq!(
            store.thread(Cid(1)).unwrap().msgs[0].send,
            SendState::Delivered
        );
    }

    #[test]
    fn contact_joined_creates_and_profile_renames() {
        let mut store = Store::new();
        assert!(store.ensure_contact(Cid(7), "Гость"));
        assert!(store.contact(Cid(7)).is_some());
        assert!(
            !store.ensure_contact(Cid(7), "Дубль"),
            "существующий не трогаем"
        );

        store.rename_contact(Cid(7), "Настоящее имя");
        assert_eq!(store.contact(Cid(7)).unwrap().name, "Настоящее имя");
    }

    #[test]
    fn block_unblock_sets_flag() {
        let mut store = Store::new();
        store.replace_contacts(vec![contact(1, "Аня", true)]);
        assert!(!store.contact(Cid(1)).unwrap().blocked);

        store.set_blocked(Cid(1), true);
        assert!(store.contact(Cid(1)).unwrap().blocked);
        store.set_blocked(Cid(1), false);
        assert!(!store.contact(Cid(1)).unwrap().blocked);
        // Неизвестный cid молча игнорируется — это данные, не баг.
        store.set_blocked(Cid(99), true);
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
