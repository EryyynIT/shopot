//! Заглушка ядра: демо-сцена, автоответы, флип presence.
//!
//! Не тестовый костыль, а равноправная реализация [`CoreLink`]
//! (`--mock`): семантика повторяет реальный канал — офлайн-отправка
//! уходит в outbox (queued), ACK приходит отдельно (доставлено).

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_core::Stream;
use std::future::Future;
use tokio::sync::mpsc;
use tokio::time;

use crate::core_link::api::Status;
use crate::core_link::{CoreLink, HistoryMsg, LinkError, LinkEvent, OkReply, Reply, Request};
use crate::model::cid::Cid;
use crate::model::msg::CoreMsgId;
use crate::model::store::Contact;

/// msg_id из номера — детерминированные 16 байт для демо/тестов.
fn core_id(n: u64) -> CoreMsgId {
    let mut bytes = [0u8; 16];
    bytes[8..].copy_from_slice(&n.to_be_bytes());
    CoreMsgId(bytes)
}

/// Сцена заглушки: кто в контактах и что происходит по таймеру.
#[derive(Debug, Clone)]
pub struct Scene {
    pub contacts: Vec<SceneContact>,
    /// Отложенные события от старта подписки.
    pub scripted: Vec<(Duration, LinkEvent)>,
    /// Флип presence случайного контакта каждые N (демо: 8 с).
    pub flip_presence: Option<Duration>,
    /// Задержка автоответа (демо: ~700 мс, 650–850 с джиттером).
    pub reply_delay_ms: u64,
    /// Реплики автоответа по кругу; в тестах — детерминированные.
    pub replies: Vec<String>,
    /// Что ядро «помнит» по контактам: ответ на GET_HISTORY. ts задан
    /// при конструировании сцены — для демо-сцены это «вчера».
    pub history: HashMap<i64, Vec<HistoryMsg>>,
}

#[derive(Debug, Clone)]
pub struct SceneContact {
    pub cid: i64,
    pub name: String,
    pub online: bool,
}

impl Scene {
    /// Демо-сцена: Аня с непрочитанными, Борис офлайн, Вера онлайн.
    pub fn demo() -> Self {
        Self {
            contacts: vec![
                SceneContact {
                    cid: 1,
                    name: "Аня".into(),
                    online: true,
                },
                SceneContact {
                    cid: 2,
                    name: "Борис".into(),
                    online: false,
                },
                SceneContact {
                    cid: 3,
                    name: "Вера".into(),
                    online: true,
                },
            ],
            scripted: vec![
                (
                    Duration::from_millis(150),
                    LinkEvent::MessageReceived {
                        cid: Cid(1),
                        name: "Аня".into(),
                        msg_id: core_id(1001),
                        ts_ms: crate::core_link::now_ms(),
                        text: "привет!".into(),
                    },
                ),
                (
                    Duration::from_millis(250),
                    LinkEvent::MessageReceived {
                        cid: Cid(1),
                        name: "Аня".into(),
                        msg_id: core_id(1002),
                        ts_ms: crate::core_link::now_ms(),
                        text: "как дела?".into(),
                    },
                ),
                (
                    Duration::from_millis(350),
                    LinkEvent::MessageReceived {
                        cid: Cid(2),
                        name: "Борис".into(),
                        msg_id: core_id(1003),
                        ts_ms: crate::core_link::now_ms(),
                        text: "есть минута?".into(),
                    },
                ),
                (
                    // Жё и Шё в кадре — проверка кириллицы жирных глифов
                    // на скриншоте (GUI_DEV.md §3).
                    Duration::from_millis(450),
                    LinkEvent::MessageReceived {
                        cid: Cid(1),
                        name: "Аня".into(),
                        msg_id: core_id(1004),
                        ts_ms: crate::core_link::now_ms(),
                        text: "Жёсткий дедлайн. Шёпотом: переждём.".into(),
                    },
                ),
            ],
            flip_presence: Some(Duration::from_secs(8)),
            reply_delay_ms: 700,
            replies: demo_replies(),
            history: demo_history(),
        }
    }

    /// Детерминированная сцена для UI-тестов: один онлайн-контакт,
    /// без флипа presence, быстрый автоответ.
    pub fn smoke() -> Self {
        Self {
            contacts: vec![SceneContact {
                cid: 1,
                name: "Аня".into(),
                online: true,
            }],
            scripted: vec![(
                Duration::from_millis(300),
                LinkEvent::MessageReceived {
                    cid: Cid(1),
                    name: "Аня".into(),
                    msg_id: core_id(2001),
                    ts_ms: crate::core_link::now_ms(),
                    text: "пинг".into(),
                },
            )],
            flip_presence: None,
            reply_delay_ms: 100,
            replies: vec!["принято".into()],
            history: HashMap::new(),
        }
    }
}

/// Вчерашняя переписка для демо-сцены: у Ани — диалог с доставкой,
/// у Бориса — одно своё офлайн-сообщение без ACK.
fn demo_history() -> HashMap<i64, Vec<HistoryMsg>> {
    let day = 24 * 60 * 60 * 1000;
    let yesterday = crate::core_link::now_ms() - day;
    HashMap::from([
        (
            1,
            vec![
                HistoryMsg {
                    mine: false,
                    delivered: false,
                    msg_id: core_id(900),
                    ts_ms: yesterday,
                    text: "слушай, ты вчера был на созвоне?".into(),
                },
                HistoryMsg {
                    mine: true,
                    delivered: true,
                    msg_id: core_id(901),
                    ts_ms: yesterday + 60_000,
                    text: "был. договорились о сроках".into(),
                },
                HistoryMsg {
                    mine: false,
                    delivered: false,
                    msg_id: core_id(902),
                    ts_ms: yesterday + 120_000,
                    text: "отлично, тогда жду макет".into(),
                },
            ],
        ),
        (
            2,
            vec![HistoryMsg {
                mine: true,
                delivered: false,
                msg_id: core_id(910),
                ts_ms: yesterday + 3_600_000,
                text: "напиши, как доберёшься".into(),
            }],
        ),
    ])
}

/// Заглушка [`crate::core_link::CoreLink`].
pub struct MockLink {
    inner: Arc<Inner>,
    scene: Arc<Scene>,
}

struct Inner {
    state: Mutex<State>,
    tx: mpsc::Sender<LinkEvent>,
    rx: Mutex<Option<mpsc::Receiver<LinkEvent>>>,
    msg_seq: AtomicU64,
    /// Очередь офлайн-отправок по контактам: уйдёт «доставлено» при флипе.
    queued: Mutex<HashMap<Cid, Vec<CoreMsgId>>>,
}

struct State {
    contacts: Vec<MockContact>,
    lcg: Lcg,
    guest_seq: u64,
}

struct MockContact {
    cid: Cid,
    name: String,
    online: bool,
    blocked: bool,
}

/// Детерминированный «шум» без внешнего крейта: джиттер задержки и
/// выбор реплики автоответа.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }
}

const REPLIES: [&str; 5] = ["ок", "принято", "понял", "согласен", "давай"];

fn demo_replies() -> Vec<String> {
    REPLIES.into_iter().map(str::to_owned).collect()
}

impl MockLink {
    pub fn new(scene: Scene) -> Self {
        let (tx, rx) = mpsc::channel(128);
        let contacts = scene
            .contacts
            .iter()
            .map(|c| MockContact {
                cid: Cid(c.cid),
                name: c.name.clone(),
                online: c.online,
                blocked: false,
            })
            .collect();
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State {
                    contacts,
                    lcg: Lcg(0x5e0f),
                    guest_seq: 0,
                }),
                tx,
                rx: Mutex::new(Some(rx)),
                msg_seq: AtomicU64::new(1),
                queued: Mutex::new(HashMap::new()),
            }),
            scene: Arc::new(scene),
        }
    }
}

impl Hash for MockLink {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        Arc::as_ptr(&self.inner).hash(hasher);
    }
}

impl CoreLink for MockLink {
    fn events(self: Arc<Self>) -> impl Stream<Item = LinkEvent> + Send + 'static {
        let Some(rx) = self.inner.rx.lock().expect("rx не занят").take() else {
            panic!("подписка на события заглушки уже взята");
        };

        // Сценарные события.
        for (delay, event) in self.scene.scripted.clone() {
            let tx = self.inner.tx.clone();
            tokio::spawn(async move {
                time::sleep(delay).await;
                let _ = tx.send(event).await;
            });
        }

        // Флип presence случайного контакта.
        if let Some(period) = self.scene.flip_presence {
            let inner = Arc::clone(&self.inner);
            tokio::spawn(async move {
                let mut ticker = time::interval(period);
                ticker.set_missed_tick_behavior(time::MissedTickBehavior::Delay);
                ticker.tick().await; // первый тик мгновенный — пропускаем
                loop {
                    ticker.tick().await;
                    for event in flip_random(&inner) {
                        let _ = inner.tx.send(event).await;
                    }
                }
            });
        }

        super::RecvStream(rx)
    }

    fn execute(&self, request: Request) -> impl Future<Output = Reply> + Send + 'static {
        let inner = Arc::clone(&self.inner);
        let scene = Arc::clone(&self.scene);
        async move { Self::handle(&inner, request, &scene).await }
    }

    fn is_stub(&self) -> bool {
        true
    }
}

impl MockLink {
    async fn handle(inner: &Arc<Inner>, request: Request, scene: &Scene) -> Reply {
        let reply_delay_ms = scene.reply_delay_ms;
        let scene_replies = &scene.replies;
        match request {
            Request::Contacts => {
                let state = inner.state.lock().expect("стейт не отравлен");
                let contacts = state
                    .contacts
                    .iter()
                    .map(|c| Contact {
                        cid: c.cid,
                        name: c.name.clone(),
                        online: c.online,
                        blocked: c.blocked,
                    })
                    .collect();
                Ok(OkReply::Contacts(contacts))
            }
            Request::SendMsg { cid, text: _ } => {
                let (online, name) = {
                    let state = inner.state.lock().expect("стейт не отравлен");
                    match state.contacts.iter().find(|c| c.cid == cid) {
                        None => return Err(LinkError::Kernel(Status::NoContact)),
                        Some(c) if c.blocked => {
                            return Err(LinkError::Kernel(Status::ContactBlocked));
                        }
                        Some(c) => (c.online, c.name.clone()),
                    }
                };

                let msg_id = core_id(inner.msg_seq.fetch_add(1, Ordering::Relaxed));
                let tx = inner.tx.clone();

                if !online {
                    // Семантика ядра: офлайн — не ошибка, а outbox.
                    inner
                        .queued
                        .lock()
                        .expect("queue не отравлен")
                        .entry(cid)
                        .or_default()
                        .push(msg_id);
                    return Ok(OkReply::Sent {
                        msg_id,
                        queued: true,
                    });
                }

                let (delay, reply_text) = {
                    let mut state = inner.state.lock().expect("стейт не отравлен");
                    let jitter = state.lcg.next() % 200;
                    let reply =
                        scene_replies[(state.lcg.next() as usize) % scene_replies.len()].clone();
                    (
                        Duration::from_millis(
                            reply_delay_ms.saturating_add(jitter).saturating_sub(100),
                        ),
                        reply,
                    )
                };
                tokio::spawn(async move {
                    // ACK от пира, затем автоответ — как живой цикл доставки.
                    time::sleep(delay / 4).await;
                    let _ = tx.send(LinkEvent::MessageDelivered { cid, msg_id }).await;
                    time::sleep(delay).await;
                    let _ = tx
                        .send(LinkEvent::MessageReceived {
                            cid,
                            name,
                            msg_id,
                            ts_ms: crate::core_link::now_ms(),
                            text: reply_text,
                        })
                        .await;
                });
                Ok(OkReply::Sent {
                    msg_id,
                    queued: false,
                })
            }
            Request::Invite { relay, addr } => {
                let mut state = inner.state.lock().expect("стейт не отравлен");
                let code: String = (0..80)
                    .map(|_| B64URL[(state.lcg.next() % 64) as usize] as char)
                    .collect();
                let host = if addr.is_empty() {
                    "192.168.1.5".to_owned()
                } else {
                    addr
                };
                let head = if relay { "shopot1r." } else { "shopot1." };
                Ok(OkReply::Code(format!("{head}{code}@{host}")))
            }
            Request::Add { code } => {
                if !code.starts_with("shopot1.") && !code.starts_with("shopot://i/") {
                    return Err(LinkError::Kernel(Status::BadInviteCode));
                }
                let (cid, guest_name) = {
                    let mut state = inner.state.lock().expect("стейт не отравлен");
                    state.guest_seq += 1;
                    let guest = state.guest_seq;
                    let cid = Cid(100 + guest as i64);
                    let guest_name = format!("Гость {guest}");
                    state.contacts.push(MockContact {
                        cid,
                        name: guest_name.clone(),
                        online: false,
                        blocked: false,
                    });
                    (cid, guest_name)
                };
                let tx = inner.tx.clone();
                tokio::spawn(async move {
                    time::sleep(Duration::from_millis(400)).await;
                    let _ = tx
                        .send(LinkEvent::ContactJoined {
                            cid,
                            name: guest_name,
                        })
                        .await;
                });
                Ok(OkReply::Added { cid })
            }
            Request::Block { cid } | Request::Unblock { cid } => {
                let blocked = matches!(request, Request::Block { .. });
                let mut state = inner.state.lock().expect("стейт не отравлен");
                match state.contacts.iter_mut().find(|c| c.cid == cid) {
                    Some(contact) => contact.blocked = blocked,
                    None => return Err(LinkError::Kernel(Status::NoContact)),
                }
                Ok(OkReply::Done)
            }
            Request::History { cid, limit } => {
                let known = {
                    let state = inner.state.lock().expect("стейт не отравлен");
                    state.contacts.iter().any(|c| c.cid == cid)
                };
                if !known {
                    return Err(LinkError::Kernel(Status::NoContact));
                }
                // Семантика ядра: окно новейших записей, старые → новые.
                let all = scene.history.get(&cid.0);
                let total = all.map_or(0, |h| h.len() as u32);
                let skip = all.map_or(0, |h| h.len().saturating_sub(limit as usize));
                let records = all.map_or_else(Vec::new, |h| h[skip..].to_vec());
                Ok(OkReply::History { total, records })
            }
        }
    }
}

fn flip_random(inner: &Arc<Inner>) -> Vec<LinkEvent> {
    let index = {
        let mut state = inner.state.lock().expect("стейт не отравлен");
        if state.contacts.is_empty() {
            return Vec::new();
        }
        (state.lcg.next() as usize) % state.contacts.len()
    };
    flip_contact(inner, index)
}

/// Флип presence контакта по индексу:Presence + квази-ACK очереди при
/// выходе в онлайн. Индекс наружу — для детерминированных тестов.
fn flip_contact(inner: &Arc<Inner>, index: usize) -> Vec<LinkEvent> {
    let mut state = inner.state.lock().expect("стейт не отравлен");
    let Some(contact) = state.contacts.get_mut(index) else {
        return Vec::new();
    };
    contact.online = !contact.online;
    let online = contact.online;
    let cid = contact.cid;
    let mut events = vec![LinkEvent::Presence {
        cid,
        online,
        last_seen_ms: 0,
    }];
    if online {
        // Квази-ACK: outbox контакта доехал.
        let delivered = inner
            .queued
            .lock()
            .expect("queue не отравлен")
            .remove(&cid)
            .unwrap_or_default();
        for msg_id in delivered {
            events.push(LinkEvent::MessageDelivered { cid, msg_id });
        }
    }
    events
}

const B64URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Хелпер тестов: собрать первые `n` событий из потока.
#[cfg(test)]
pub(crate) use super::take_events;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core_link::CoreLink;

    fn scene() -> Scene {
        Scene {
            contacts: vec![
                SceneContact {
                    cid: 1,
                    name: "Аня".into(),
                    online: true,
                },
                SceneContact {
                    cid: 2,
                    name: "Борис".into(),
                    online: false,
                },
            ],
            scripted: vec![],
            flip_presence: None,
            reply_delay_ms: 50,
            replies: demo_replies(),
            history: HashMap::new(),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn contacts_are_typed() {
        let link = MockLink::new(scene());
        let reply = link.execute(Request::Contacts).await;
        let Ok(OkReply::Contacts(contacts)) = reply else {
            panic!("{reply:?}");
        };
        assert_eq!(contacts.len(), 2);
        assert_eq!(contacts[0].cid, Cid(1));
        assert_eq!(contacts[0].name, "Аня");
        assert!(contacts[0].online);
        assert!(!contacts[1].blocked);
    }

    #[tokio::test(start_paused = true)]
    async fn send_to_online_contact_delivers_and_autoreplies() {
        let link = Arc::new(MockLink::new(scene()));

        let reply = link
            .execute(Request::SendMsg {
                cid: Cid(1),
                text: "привет".into(),
            })
            .await;
        let Ok(OkReply::Sent { queued, .. }) = reply else {
            panic!("{reply:?}");
        };
        assert!(!queued, "онлайн-контакт получает сразу");

        let mut events = take_events(CoreLink::events(Arc::clone(&link)), 2).await;
        assert!(
            matches!(
                events.pop_front(),
                Some(LinkEvent::MessageDelivered { cid: Cid(1), .. })
            ),
            "сначала ACK доставки"
        );
        let event = events.pop_front().expect("автоответ приходит");
        let LinkEvent::MessageReceived { cid, text, .. } = event else {
            panic!("ожидали MessageReceived, получено {event:?}");
        };
        assert_eq!(cid, Cid(1));
        assert!(!text.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn send_to_offline_contact_is_queued_until_flip() {
        let link = Arc::new(MockLink::new(scene()));

        let reply = link
            .execute(Request::SendMsg {
                cid: Cid(2),
                text: "привет".into(),
            })
            .await;
        let Ok(OkReply::Sent { queued, .. }) = reply else {
            panic!("{reply:?}");
        };
        assert!(queued, "офлайн — это outbox, не ошибка");

        // Борис выходит в онлайн — квази-ACK: очередь доехала.
        for event in flip_contact(&link.inner, 1) {
            let _ = link.inner.tx.send(event).await;
        }
        let mut events = take_events(CoreLink::events(Arc::clone(&link)), 2).await;
        assert!(matches!(
            events.pop_front(),
            Some(LinkEvent::Presence { online: true, .. })
        ));
        assert!(matches!(
            events.pop_front(),
            Some(LinkEvent::MessageDelivered { cid: Cid(2), .. })
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn add_valid_code_creates_contact_and_announces_join() {
        let link = Arc::new(MockLink::new(scene()));

        let reply = link
            .execute(Request::Add {
                code: "shopot1.abc@1.2.3.4".into(),
            })
            .await;
        let Ok(OkReply::Added { cid }) = reply else {
            panic!("{reply:?}");
        };
        assert_eq!(cid, Cid(101));

        let mut events = take_events(CoreLink::events(Arc::clone(&link)), 1).await;
        let event = events.pop_front().expect("ContactJoined приходит");
        assert_eq!(
            event,
            LinkEvent::ContactJoined {
                cid: Cid(101),
                name: "Гость 1".into()
            }
        );

        let Ok(OkReply::Contacts(contacts)) = link.execute(Request::Contacts).await else {
            panic!();
        };
        assert!(contacts.iter().any(|c| c.name == "Гость 1"));
    }

    #[tokio::test(start_paused = true)]
    async fn add_bad_code_is_kernel_error() {
        let link = MockLink::new(scene());
        let reply = link
            .execute(Request::Add {
                code: "mqtt://broker".into(),
            })
            .await;
        assert_eq!(reply, Err(LinkError::Kernel(Status::BadInviteCode)));
        assert!(reply.unwrap_err().user_text().contains("Скопируйте"));
    }

    #[tokio::test(start_paused = true)]
    async fn invite_code_has_v2_shape() {
        let link = MockLink::new(scene());
        let reply = link
            .execute(Request::Invite {
                relay: false,
                addr: String::new(),
            })
            .await;
        let Ok(OkReply::Code(code)) = reply else {
            panic!()
        };
        assert!(code.starts_with("shopot1."));
        assert!(code.ends_with("@192.168.1.5"));
    }

    #[tokio::test(start_paused = true)]
    async fn block_marks_contact() {
        let link = MockLink::new(scene());
        assert_eq!(
            link.execute(Request::Block { cid: Cid(1) }).await,
            Ok(OkReply::Done)
        );
        let Ok(OkReply::Contacts(contacts)) = link.execute(Request::Contacts).await else {
            panic!();
        };
        assert!(contacts[0].blocked);
        assert_eq!(
            link.execute(Request::Unblock { cid: Cid(1) }).await,
            Ok(OkReply::Done)
        );
        let Ok(OkReply::Contacts(contacts)) = link.execute(Request::Contacts).await else {
            panic!();
        };
        assert!(!contacts[0].blocked);
    }

    #[tokio::test(start_paused = true)]
    async fn unknown_contact_send_is_kernel_no_contact() {
        let link = MockLink::new(scene());
        let reply = link
            .execute(Request::SendMsg {
                cid: Cid(99),
                text: "эй".into(),
            })
            .await;
        assert_eq!(reply, Err(LinkError::Kernel(Status::NoContact)));
        assert_eq!(reply.unwrap_err().user_text(), "Контакт не найден.");
    }

    /// Окно GET_HISTORY: новейшие `limit` записей, порядок старые → новые,
    /// total не зависит от окна.
    #[tokio::test(start_paused = true)]
    async fn history_returns_newest_window_in_order() {
        let mut scene = scene();
        scene.history.insert(
            1,
            (0..5)
                .map(|n| HistoryMsg {
                    mine: false,
                    delivered: false,
                    msg_id: core_id(n),
                    ts_ms: n as i64 * 1000,
                    text: format!("строка {n}"),
                })
                .collect(),
        );
        let link = MockLink::new(scene);

        let reply = link
            .execute(Request::History {
                cid: Cid(1),
                limit: 2,
            })
            .await;
        assert_eq!(
            reply,
            Ok(OkReply::History {
                total: 5,
                records: vec![record(3, "строка 3"), record(4, "строка 4"),],
            }),
            "окно = две новейшие, порядок не перевёрнут"
        );
        // Лимит больше размера — отдаётся всё.
        let reply = link
            .execute(Request::History {
                cid: Cid(1),
                limit: 50,
            })
            .await;
        assert!(matches!(
            reply,
            Ok(OkReply::History { total: 5, records }) if records.len() == 5
        ));
        // Пустая история — валидный пустой ответ.
        let reply = link
            .execute(Request::History {
                cid: Cid(2),
                limit: 50,
            })
            .await;
        assert_eq!(
            reply,
            Ok(OkReply::History {
                total: 0,
                records: vec![]
            })
        );
    }

    fn record(n: u64, text: &str) -> HistoryMsg {
        HistoryMsg {
            mine: false,
            delivered: false,
            msg_id: core_id(n),
            ts_ms: n as i64 * 1000,
            text: text.to_owned(),
        }
    }
}
