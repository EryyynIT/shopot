//! Заглушка ядра: демо-сцена, автоответы, флип presence.
//!
//! Не тестовый костыль, а равноправная реализация [`CoreLink`]:
//! она же демо-режим (статус-строка «режим: заглушка»).

use std::hash::{Hash, Hasher};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use futures_core::Stream;
use std::future::Future;
use tokio::sync::mpsc;
use tokio::time;

use crate::core_link::{LinkEvent, Reply, Request};
use crate::model::cid::Cid;
use crate::model::msg::MsgId;

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
        use crate::model::msg::MsgId;
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
                    LinkEvent::IMsg {
                        cid: Cid(1),
                        name: "Аня".into(),
                        msg_id: MsgId(1001),
                        ts_ms: crate::core_link::now_ms(),
                        text: "привет!".into(),
                    },
                ),
                (
                    Duration::from_millis(250),
                    LinkEvent::IMsg {
                        cid: Cid(1),
                        name: "Аня".into(),
                        msg_id: MsgId(1002),
                        ts_ms: crate::core_link::now_ms(),
                        text: "как дела?".into(),
                    },
                ),
                (
                    Duration::from_millis(350),
                    LinkEvent::IMsg {
                        cid: Cid(2),
                        name: "Борис".into(),
                        msg_id: MsgId(1003),
                        ts_ms: crate::core_link::now_ms(),
                        text: "есть минута?".into(),
                    },
                ),
            ],
            flip_presence: Some(Duration::from_secs(8)),
            reply_delay_ms: 700,
            replies: demo_replies(),
        }
    }

    /// Детерминированная сцена для UI-тестов: один онлайн-контакт,
    /// без флипа presence, быстрый автоответ.
    pub fn smoke() -> Self {
        use crate::model::msg::MsgId;
        Self {
            contacts: vec![SceneContact {
                cid: 1,
                name: "Аня".into(),
                online: true,
            }],
            scripted: vec![(
                Duration::from_millis(300),
                LinkEvent::IMsg {
                    cid: Cid(1),
                    name: "Аня".into(),
                    msg_id: MsgId(2001),
                    ts_ms: crate::core_link::now_ms(),
                    text: "пинг".into(),
                },
            )],
            flip_presence: None,
            reply_delay_ms: 100,
            replies: vec!["принято".into()],
        }
    }
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

impl super::CoreLink for MockLink {
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

        RecvStream(rx)
    }

    fn execute(&self, request: Request) -> impl Future<Output = Reply> + Send + 'static {
        let inner = Arc::clone(&self.inner);
        let reply_delay_ms = self.scene.reply_delay_ms;
        let scene_replies = self.scene.replies.clone();
        async move { Self::handle(&inner, request, reply_delay_ms, &scene_replies).await }
    }
}

impl MockLink {
    async fn handle(
        inner: &Arc<Inner>,
        request: Request,
        reply_delay_ms: u64,
        scene_replies: &[String],
    ) -> Reply {
        match request {
            Request::Contacts => {
                let state = inner.state.lock().expect("стейт не отравлен");
                let lines = state
                    .contacts
                    .iter()
                    .map(|c| {
                        crate::core_link::format_contact(&crate::model::store::Contact {
                            cid: c.cid,
                            name: c.name.clone(),
                            online: c.online,
                            blocked: c.blocked,
                        })
                    })
                    .collect();
                Reply::Ok(lines)
            }
            Request::SendMsg { cid, text: _ } => {
                let (online, name) = {
                    let state = inner.state.lock().expect("стейт не отравлен");
                    match state.contacts.iter().find(|c| c.cid == cid) {
                        None => return Reply::Err("нет такого контакта".into()),
                        Some(c) if c.blocked => {
                            return Reply::Err(
                                "контакт заблокирован: разблокируйте, чтобы писать".into(),
                            )
                        }
                        Some(c) => (c.online, c.name.clone()),
                    }
                };
                if !online {
                    return Reply::Err(
                        "Нет связи с пиром. Сообщение не отправлено — попробуйте позже.".into(),
                    );
                }

                let tx = inner.tx.clone();
                let msg_id = next_msg_id(&inner.msg_seq);
                let delay = {
                    let mut state = inner.state.lock().expect("стейт не отравлен");
                    let jitter = state.lcg.next() % 200;
                    Duration::from_millis(reply_delay_ms.saturating_add(jitter).saturating_sub(100))
                };
                let reply_text = {
                    let mut state = inner.state.lock().expect("стейт не отравлен");
                    scene_replies[(state.lcg.next() as usize) % scene_replies.len()].clone()
                };
                tokio::spawn(async move {
                    time::sleep(delay).await;
                    let _ = tx
                        .send(LinkEvent::IMsg {
                            cid,
                            name,
                            msg_id,
                            ts_ms: crate::core_link::now_ms(),
                            text: reply_text,
                        })
                        .await;
                });
                Reply::Ok(vec![])
            }
            Request::Invite => {
                let mut state = inner.state.lock().expect("стейт не отравлен");
                let code: String = (0..80)
                    .map(|_| B64URL[(state.lcg.next() % 64) as usize] as char)
                    .collect();
                Reply::Ok(vec![format!("shopot1.{code}@192.168.1.5")])
            }
            Request::Add { code } => {
                if !code.starts_with("shopot1.") {
                    return Reply::Err(
                        "Неверный код. Скопируйте строку целиком — она начинается с shopot1."
                            .into(),
                    );
                }
                let cid = {
                    let mut state = inner.state.lock().expect("стейт не отравлен");
                    state.guest_seq += 1;
                    let guest = state.guest_seq;
                    let cid = Cid(100 + guest as i64);
                    let name = format!("Гость {guest}");
                    state.contacts.push(MockContact {
                        cid,
                        name,
                        online: false,
                        blocked: false,
                    });
                    cid
                };
                let tx = inner.tx.clone();
                tokio::spawn(async move {
                    time::sleep(Duration::from_millis(400)).await;
                    let _ = tx.send(LinkEvent::Presence { cid, online: true }).await;
                    let _ = tx
                        .send(LinkEvent::Sys {
                            cid,
                            text: "сессия установлена".into(),
                        })
                        .await;
                });
                Reply::Ok(vec![])
            }
            Request::Block { cid } | Request::Unblock { cid } => {
                let blocked = matches!(request, Request::Block { .. });
                let mut state = inner.state.lock().expect("стейт не отравлен");
                match state.contacts.iter_mut().find(|c| c.cid == cid) {
                    Some(contact) => contact.blocked = blocked,
                    None => return Reply::Err("нет такого контакта".into()),
                }
                Reply::Ok(vec![])
            }
        }
    }
}

fn next_msg_id(seq: &AtomicU64) -> MsgId {
    MsgId(seq.fetch_add(1, Ordering::Relaxed))
}

fn flip_random(inner: &Arc<Inner>) -> Vec<LinkEvent> {
    let mut state = inner.state.lock().expect("стейт не отравлен");
    if state.contacts.is_empty() {
        return Vec::new();
    }
    let index = (state.lcg.next() as usize) % state.contacts.len();
    let contact = &mut state.contacts[index];
    contact.online = !contact.online;
    let online = contact.online;
    let cid = contact.cid;
    let mut events = vec![LinkEvent::Presence { cid, online }];
    if online {
        events.push(LinkEvent::Sys {
            cid,
            text: "сессия установлена".into(),
        });
    }
    events
}

const B64URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Мост tokio mpsc → futures Stream (без tokio-stream: poll_recv есть публично).
struct RecvStream(mpsc::Receiver<LinkEvent>);

impl Stream for RecvStream {
    type Item = LinkEvent;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.0.poll_recv(cx)
    }
}
/// Хелпер тестов: собрать первые `n` событий из потока.
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
    use crate::core_link::CoreLink;
    use crate::model::msg::MsgId;

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
        }
    }

    #[tokio::test(start_paused = true)]
    async fn contacts_lines_follow_machine_format() {
        let link = MockLink::new(scene());
        let reply = link.execute(Request::Contacts).await;
        assert_eq!(
            reply,
            Reply::Ok(vec!["c 1 on - Аня".into(), "c 2 off - Борис".into(),])
        );
    }

    #[tokio::test(start_paused = true)]
    async fn send_to_online_contact_autoreplies() {
        let link = Arc::new(MockLink::new(scene()));

        let reply = link
            .execute(Request::SendMsg {
                cid: Cid(1),
                text: "привет".into(),
            })
            .await;
        assert_eq!(reply, Reply::Ok(vec![]));

        let mut events = take_events(CoreLink::events(Arc::clone(&link)), 1).await;

        let event = events.pop_front().expect("автоответ приходит");
        let LinkEvent::IMsg { cid, text, .. } = event else {
            panic!("ожидали IMsg, получено {event:?}");
        };
        assert_eq!(cid, Cid(1));
        assert!(!text.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn send_to_offline_contact_fails_honestly() {
        let link = MockLink::new(scene());
        let reply = link
            .execute(Request::SendMsg {
                cid: Cid(2),
                text: "привет".into(),
            })
            .await;
        assert!(matches!(reply, Reply::Err(_)));
    }

    #[tokio::test(start_paused = true)]
    async fn add_valid_code_creates_contact_and_comes_online() {
        let link = Arc::new(MockLink::new(scene()));

        let reply = link
            .execute(Request::Add {
                code: "shopot1.abc@1.2.3.4".into(),
            })
            .await;
        assert!(matches!(reply, Reply::Ok(_)));

        let mut events = take_events(CoreLink::events(Arc::clone(&link)), 2).await;

        let lines = match link.execute(Request::Contacts).await {
            Reply::Ok(lines) => lines,
            other => panic!("{other:?}"),
        };
        assert!(
            lines.iter().any(|l| l.contains("Гость 1")),
            "строки контактов: {lines:?}"
        );

        // Гость выходит в линк и здоровается служебной строкой.
        let presence = events.pop_front().expect("presence");
        assert_eq!(
            presence,
            LinkEvent::Presence {
                cid: Cid(101),
                online: true
            }
        );
        let sys = events.pop_front().expect("sys");
        assert_eq!(
            sys,
            LinkEvent::Sys {
                cid: Cid(101),
                text: "сессия установлена".into()
            }
        );
    }

    #[tokio::test(start_paused = true)]
    async fn add_bad_code_is_an_error_with_hint() {
        let link = MockLink::new(scene());
        let reply = link
            .execute(Request::Add {
                code: "mqtt://broker".into(),
            })
            .await;
        assert!(matches!(reply, Reply::Err(_)));
    }

    #[tokio::test(start_paused = true)]
    async fn invite_code_has_v2_shape() {
        let link = MockLink::new(scene());
        let reply = link.execute(Request::Invite).await;
        let Reply::Ok(lines) = reply else { panic!() };
        assert_eq!(lines.len(), 1);
        assert!(lines[0].starts_with("shopot1."));
        assert!(lines[0].ends_with("@192.168.1.5"));
    }

    #[tokio::test(start_paused = true)]
    async fn block_marks_contact() {
        let link = MockLink::new(scene());
        link.execute(Request::Block { cid: Cid(1) }).await;
        let Reply::Ok(lines) = link.execute(Request::Contacts).await else {
            panic!()
        };
        assert!(lines[0].contains(" b "));
        link.execute(Request::Unblock { cid: Cid(1) }).await;
        let Reply::Ok(lines) = link.execute(Request::Contacts).await else {
            panic!()
        };
        assert!(lines[0].contains(" - "));
    }

    #[test]
    fn msg_ids_render_hex16() {
        assert_eq!(MsgId(255).to_string().len(), 16);
        assert_eq!(MsgId(0).to_string(), "0".repeat(16));
    }
}
