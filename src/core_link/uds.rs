//! UDS-транспорт бинарного API 1.0.0: соединение с демоном `shopot serve`
//! (`$HOME/.shopot/api.sock`), корреляция ответов по request_id, события
//! в поток, backoff-ресинк при обрыве. Здесь же раскладки payload методов
//! и событий — структурный слой поверх кодека [`super::api`].

use std::collections::HashMap;
use std::future::Future;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_core::Stream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::{mpsc, oneshot};
use tokio::time::timeout;

use super::api::{self, Header, Kind, Method, Status};
use super::{CoreLink, LinkError, LinkEvent, OkReply, RecvStream, Reply, Request};
use crate::model::cid::Cid;
use crate::model::msg::CoreMsgId;
use crate::model::store::Contact;

/// Сколько ждать ответа ядра, прежде чем считать запрос зависшим.
const REPLY_TIMEOUT: Duration = Duration::from_secs(15);
/// Потолок backoff-ресинка (0.5 → 1 → … → 30 с).
const MAX_BACKOFF: Duration = Duration::from_secs(30);
/// Ёмкость очереди исходящих кадров и событий.
const CHANNEL_CAP: usize = 256;

/// Ждущие ответа: request_id → канал ответа. Arc — читатель живёт
/// в отдельной задаче и не заимствует Inner.
type Pending = Arc<Mutex<HashMap<u32, oneshot::Sender<Result<RawReply, LinkError>>>>>;

/// Сырой ответ ядра: статус + payload; раскладку знает только decode_reply.
struct RawReply {
    status: Status,
    payload: Vec<u8>,
}

/// Реальный канал с ядром: демон `shopot serve` обязан быть запущен
/// (при отсутствии GUI сам попытается его поднять — один раз).
pub struct UdsLink {
    inner: Arc<Inner>,
}

struct Inner {
    path: PathBuf,
    /// База backoff; тесты ставят маленькую.
    backoff: Duration,
    /// Пробовать спавнить `shopot serve`, если сокета нет.
    spawn_daemon: bool,
    /// Живое соединение: очередь кадров записи.
    conn: Mutex<Option<mpsc::Sender<Vec<u8>>>>,
    pending: Pending,
    next_id: AtomicU32,
    events_tx: mpsc::Sender<LinkEvent>,
    events_rx: Mutex<Option<mpsc::Receiver<LinkEvent>>>,
}

impl UdsLink {
    /// Канал к демону текущего пользователя ($HOME/.shopot/api.sock).
    pub fn new() -> Self {
        let home = std::env::var("HOME").unwrap_or_default();
        let path = Path::new(&home).join(".shopot").join("api.sock");
        Self::with_config(path, Duration::from_millis(500), true)
    }

    /// Точки входа для тестов: путь, база backoff, спавн демона.
    #[doc(hidden)]
    pub fn with_config(path: PathBuf, backoff: Duration, spawn_daemon: bool) -> Self {
        let (events_tx, events_rx) = mpsc::channel(CHANNEL_CAP);
        Self {
            inner: Arc::new(Inner {
                path,
                backoff,
                spawn_daemon,
                conn: Mutex::new(None),
                pending: Arc::new(Mutex::new(HashMap::new())),
                next_id: AtomicU32::new(1),
                events_tx,
                events_rx: Mutex::new(Some(events_rx)),
            }),
        }
    }
}

impl Default for UdsLink {
    fn default() -> Self {
        Self::new()
    }
}

impl Hash for UdsLink {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        Arc::as_ptr(&self.inner).hash(hasher);
    }
}

impl CoreLink for UdsLink {
    fn events(self: Arc<Self>) -> impl Stream<Item = LinkEvent> + Send + 'static {
        let Some(rx) = self.inner.events_rx.lock().expect("rx не занят").take() else {
            panic!("подписка на события канала уже взята");
        };
        let inner = Arc::clone(&self.inner);
        tokio::spawn(inner.run());
        RecvStream(rx)
    }

    fn execute(&self, request: Request) -> impl Future<Output = Reply> + Send + 'static {
        let inner = Arc::clone(&self.inner);
        async move { inner.request(request).await }
    }
}

impl Inner {
    /// Супервизор соединения: коннект → обслуживание → обрыв → backoff.
    /// Каждое успешное соединение начинается с Resync: UI перечитает
    /// контакты, лента в памяти GUI остаётся.
    async fn run(self: Arc<Self>) {
        let mut backoff = self.backoff;
        let mut daemon_spawned = false;
        loop {
            match UnixStream::connect(&self.path).await {
                Ok(stream) => {
                    backoff = self.backoff;
                    let _ = self.events_tx.send(LinkEvent::Resync).await;
                    self.serve(stream).await;
                    let _ = self.events_tx.send(LinkEvent::Down).await;
                }
                Err(_) => {
                    if self.spawn_daemon && !daemon_spawned {
                        daemon_spawned = true;
                        Self::spawn_daemon().await;
                    }
                }
            }
            tokio::time::sleep(backoff).await;
            backoff = backoff.saturating_mul(2).min(MAX_BACKOFF);
        }
    }

    /// Спавн `shopot serve` из PATH; у свежего $HOME сперва `shopot init`
    /// (демон без сида сразу выходит). GUI не становится его сидом:
    /// процесс ждут в отдельной задаче только ради reap.
    async fn spawn_daemon() {
        let home = std::env::var("HOME").unwrap_or_default();
        let seed = Path::new(&home).join(".shopot/zig-seed");
        if !seed.exists() {
            let _ = tokio::process::Command::new("shopot")
                .arg("init")
                .output()
                .await;
        }
        let Ok(mut child) = tokio::process::Command::new("shopot").arg("serve").spawn() else {
            return; // бинарника нет — статус «нет связи», не шумим
        };
        tokio::spawn(async move {
            let _ = child.wait().await;
        });
    }

    /// Обслуживание одного соединения до EOF: читатель собирает кадры,
    /// писатель сериализует записи; общий стейт — только pending.
    async fn serve(&self, stream: UnixStream) {
        let (mut read_half, mut write_half) = stream.into_split();
        let (writer_tx, mut writer_rx) = mpsc::channel::<Vec<u8>>(CHANNEL_CAP);
        *self.conn.lock().expect("conn не отравлен") = Some(writer_tx.clone());

        let pending = Arc::clone(&self.pending);
        let events_tx = self.events_tx.clone();
        let reader = tokio::spawn(async move {
            let mut buf = [0u8; api::HEADER_LEN];
            loop {
                if read_half.read_exact(&mut buf).await.is_err() {
                    break;
                }
                let Ok(header) = api::decode_header(&buf) else {
                    break; // магия/версия/длина — поток не восстановим
                };
                let mut payload = vec![0u8; header.payload_len as usize];
                if header.payload_len > 0 && read_half.read_exact(&mut payload).await.is_err() {
                    break;
                }
                match header.kind {
                    Kind::Response => resolve_pending(&pending, header, payload),
                    Kind::Event => {
                        if let Some(event) = decode_event(&header, &payload) {
                            let _ = events_tx.send(event).await;
                        }
                        // Неизвестное событие моложе клиента — пропускаем.
                    }
                    Kind::Request => break, // ядро не спрашивает клиента
                }
            }
        });

        let writer = tokio::spawn(async move {
            while let Some(frame) = writer_rx.recv().await {
                if write_half.write_all(&frame).await.is_err() {
                    break;
                }
            }
        });

        let _ = reader.await;
        writer.abort();
        *self.conn.lock().expect("conn не отравлен") = None;
        // Все зависшие запросы получают Disconnected.
        self.pending
            .lock()
            .expect("pending не отравлен")
            .drain()
            .for_each(|(_, tx)| {
                let _ = tx.send(Err(LinkError::Disconnected));
            });
    }

    async fn request(&self, request: Request) -> Reply {
        let (method, payload) = encode_request(&request);
        let request_id = self.next_id.fetch_add(1, Ordering::Relaxed);

        // Лок берём только чтобы клонировать очередь записи: никакие
        // локи не держатся через await.
        let writer = {
            let conn = self.conn.lock().expect("conn не отравлен");
            match conn.as_ref() {
                Some(writer) => writer.clone(),
                None => return Err(LinkError::Disconnected),
            }
        };

        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .expect("pending не отравлен")
            .insert(request_id, tx);

        let mut frame = Vec::with_capacity(api::HEADER_LEN + payload.len());
        api::encode_frame(
            &mut frame,
            Header {
                kind: Kind::Request,
                id: method as u16,
                request_id,
                status: Status::Ok,
                payload_len: payload.len() as u32,
            },
            &payload,
        );

        if writer.send(frame).await.is_err() {
            self.pending
                .lock()
                .expect("pending не отравлен")
                .remove(&request_id);
            return Err(LinkError::Disconnected);
        }

        match timeout(REPLY_TIMEOUT, rx).await {
            Ok(Ok(Ok(raw))) => decode_reply(method, raw),
            // Дренаж при обрыве: всем отправлен Disconnected.
            Ok(Ok(Err(err))) => Err(err),
            // Канал ответа закрыт без отправки — соединение умерло.
            Ok(Err(_dropped)) => Err(LinkError::Disconnected),
            Err(_) => {
                self.pending
                    .lock()
                    .expect("pending не отравлен")
                    .remove(&request_id);
                Err(LinkError::Disconnected)
            }
        }
    }
}

/// Ответ ядра на дождавшийся запрос.
fn resolve_pending(pending: &Pending, header: Header, payload: Vec<u8>) {
    if let Some(tx) = pending
        .lock()
        .expect("pending не отравлен")
        .remove(&header.request_id)
    {
        let _ = tx.send(Ok(RawReply {
            status: header.status,
            payload,
        }));
    }
    // Ответ без ждущего: таймаут уже истёк — молча выбрасываем.
}

/// Запрос → метод API + payload (раскладки — CORE-API.md §4).
fn encode_request(request: &Request) -> (Method, Vec<u8>) {
    let mut payload = Vec::new();
    let method = match request {
        Request::Contacts => Method::ListContacts,
        Request::SendMsg { cid, text } => {
            api::put::u32(&mut payload, cid.0 as u32);
            api::put::str16(&mut payload, text).expect("текст уже проверен на 4096");
            Method::SendMessage
        }
        // my_name пустой — ядро подставит «друг».
        Request::Add { code } => {
            api::put::str16(&mut payload, code).expect("код короче 65535");
            api::put::str16(&mut payload, "").expect("пустая строка");
            Method::AcceptInvite
        }
        // name пустой — ядро подставит «shopot»; addr пустой — LAN-анонс;
        // relay=1 требует addr: соединение идёт через релей, peer IP не видит.
        Request::Invite { relay, addr } => {
            api::put::u8(&mut payload, u8::from(*relay));
            api::put::str16(&mut payload, "").expect("пустая строка");
            api::put::str16(&mut payload, addr).expect("адрес короче 65535");
            Method::CreateInvite
        }
        Request::Block { cid } => {
            api::put::u32(&mut payload, cid.0 as u32);
            Method::BlockContact
        }
        Request::Unblock { cid } => {
            api::put::u32(&mut payload, cid.0 as u32);
            Method::UnblockContact
        }
    };
    (method, payload)
}

/// Ответ → типизированный Reply по методу; статус != OK — Err(Kernel).
fn decode_reply(method: Method, raw: RawReply) -> Reply {
    if raw.status != Status::Ok {
        return Err(LinkError::Kernel(raw.status));
    }
    let reply = match method {
        Method::ListContacts => OkReply::Contacts(decode_contacts(&raw.payload)?),
        Method::SendMessage => {
            let mut r = api::Reader::new(&raw.payload);
            let mut msg_id = [0u8; 16];
            msg_id.copy_from_slice(
                r.read_bytes(16)
                    .map_err(|_| LinkError::BadFrame("msg_id"))?,
            );
            let queued = r.read_u8().map_err(|_| LinkError::BadFrame("queued"))? == 1;
            OkReply::Sent {
                msg_id: CoreMsgId(msg_id),
                queued,
            }
        }
        Method::AcceptInvite => {
            let mut r = api::Reader::new(&raw.payload);
            let cid = r.read_u32().map_err(|_| LinkError::BadFrame("cid"))?;
            OkReply::Added {
                cid: Cid(i64::from(cid)),
            }
        }
        Method::CreateInvite => {
            let mut r = api::Reader::new(&raw.payload);
            let code = r.read_str16().map_err(|_| LinkError::BadFrame("code"))?;
            OkReply::Code(String::from_utf8_lossy(code).into_owned())
        }
        _ => OkReply::Done,
    };
    Ok(reply)
}

/// LIST_CONTACTS: count × {cid u32, flags u8, last_seen i64, name str16}.
fn decode_contacts(payload: &[u8]) -> Result<Vec<Contact>, LinkError> {
    let mut r = api::Reader::new(payload);
    let count = r.read_u32().map_err(|_| LinkError::BadFrame("count"))?;
    let mut contacts = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let cid = r.read_u32().map_err(|_| LinkError::BadFrame("cid"))?;
        let flags = r.read_u8().map_err(|_| LinkError::BadFrame("flags"))?;
        let _last_seen = r.read_i64().map_err(|_| LinkError::BadFrame("last_seen"))?;
        let name = r.read_str16().map_err(|_| LinkError::BadFrame("name"))?;
        contacts.push(Contact {
            cid: Cid(i64::from(cid)),
            name: String::from_utf8_lossy(name).into_owned(),
            online: flags & 0b01 != 0,
            blocked: flags & 0b10 != 0,
        });
    }
    Ok(contacts)
}

/// Событие → LinkEvent; не используемые клиентом события (реакции,
/// звонки, аватары, файлы) — None, до своих спринтов.
fn decode_event(header: &Header, payload: &[u8]) -> Option<LinkEvent> {
    let mut r = api::Reader::new(payload);
    match api::Event::from_u16(header.id)? {
        api::Event::MessageReceived => {
            let cid = r.read_u32().ok()?;
            let mut msg_id = [0u8; 16];
            msg_id.copy_from_slice(r.read_bytes(16).ok()?);
            let ts_ms = r.read_u64().ok()? as i64;
            let name = r.read_str16().ok()?;
            let text = r.read_str16().ok()?;
            Some(LinkEvent::MessageReceived {
                cid: Cid(i64::from(cid)),
                name: String::from_utf8_lossy(name).into_owned(),
                msg_id: CoreMsgId(msg_id),
                ts_ms,
                text: String::from_utf8_lossy(text).into_owned(),
            })
        }
        api::Event::MessageDelivered => {
            let cid = r.read_u32().ok()?;
            let mut msg_id = [0u8; 16];
            msg_id.copy_from_slice(r.read_bytes(16).ok()?);
            Some(LinkEvent::MessageDelivered {
                cid: Cid(i64::from(cid)),
                msg_id: CoreMsgId(msg_id),
            })
        }
        api::Event::Presence => {
            let cid = r.read_u32().ok()?;
            let online = r.read_u8().ok()?;
            let last_seen_ms = r.read_i64().ok()?;
            Some(LinkEvent::Presence {
                cid: Cid(i64::from(cid)),
                online: online != 0,
                last_seen_ms,
            })
        }
        api::Event::ContactJoined => {
            let cid = r.read_u32().ok()?;
            let name = r.read_str16().ok()?;
            Some(LinkEvent::ContactJoined {
                cid: Cid(i64::from(cid)),
                name: String::from_utf8_lossy(name).into_owned(),
            })
        }
        api::Event::ContactProfile => {
            let cid = r.read_u32().ok()?;
            let name = r.read_str16().ok()?;
            let mut avatar_id = [0u8; 16];
            avatar_id.copy_from_slice(r.read_bytes(16).ok()?);
            Some(LinkEvent::ContactProfile {
                cid: Cid(i64::from(cid)),
                name: String::from_utf8_lossy(name).into_owned(),
                avatar_id: CoreMsgId(avatar_id),
            })
        }
        api::Event::BlockedByPeer => {
            let cid = r.read_u32().ok()?;
            Some(LinkEvent::BlockedByPeer {
                cid: Cid(i64::from(cid)),
            })
        }
        api::Event::Reaction
        | api::Event::CallState
        | api::Event::AvatarReceived
        | api::Event::FileReceived => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core_link::take_events;
    use tokio::net::{UnixListener, UnixStream as TokioUnixStream};

    /// Путь сокета в /tmp, уникальный для теста.
    fn temp_socket(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir();
        dir.join(format!("shopot-gui-test-{tag}-{}.sock", std::process::id()))
    }

    fn response_frame(id: u16, request_id: u32, status: Status, payload: &[u8]) -> Vec<u8> {
        let mut frame = Vec::new();
        api::encode_frame(
            &mut frame,
            Header {
                kind: Kind::Response,
                id,
                request_id,
                status,
                payload_len: payload.len() as u32,
            },
            payload,
        );
        frame
    }

    fn event_frame(event: api::Event, payload: &[u8]) -> Vec<u8> {
        let mut frame = Vec::new();
        api::encode_frame(
            &mut frame,
            Header {
                kind: Kind::Event,
                id: event as u16,
                request_id: 0,
                status: Status::Ok,
                payload_len: payload.len() as u32,
            },
            payload,
        );
        frame
    }

    async fn read_frame(stream: &mut TokioUnixStream) -> (Header, Vec<u8>) {
        let mut buf = [0u8; api::HEADER_LEN];
        stream
            .read_exact(&mut buf)
            .await
            .expect("заголовок прочитан");
        let header = api::decode_header(&buf).expect("кадр корректен");
        let mut payload = vec![0u8; header.payload_len as usize];
        if header.payload_len > 0 {
            stream
                .read_exact(&mut payload)
                .await
                .expect("payload прочитан");
        }
        (header, payload)
    }

    /// Убрать сокет, если тест до него не добрался.
    fn cleanup(path: &Path) {
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn contacts_roundtrip_decodes_typed_list() {
        let path = temp_socket("contacts");
        cleanup(&path);
        let listener = UnixListener::bind(&path).expect("сокет забинден");

        let link = Arc::new(UdsLink::with_config(
            path.clone(),
            Duration::from_millis(10),
            false,
        ));
        let events = take_events(CoreLink::events(Arc::clone(&link)), 1);

        let (mut stream, _) = listener.accept().await.expect("клиент подключился");
        // Запрос в отдельной задаче: однопоточный рантайм теста —
        // главный таск обслуживает фейковое ядро.
        let reply = {
            let link = Arc::clone(&link);
            tokio::spawn(async move { CoreLink::execute(&*link, Request::Contacts).await })
        };

        let (header, payload) = read_frame(&mut stream).await;
        assert_eq!(header.kind, Kind::Request);
        assert_eq!(header.id, Method::ListContacts as u16);
        assert_eq!(payload.len(), 0, "LIST_CONTACTS без payload");

        let mut answer = Vec::new();
        api::put::u32(&mut answer, 1);
        api::put::u32(&mut answer, 5);
        api::put::u8(&mut answer, 0b01);
        api::put::i64(&mut answer, 1234);
        api::put::str16(&mut answer, "Аня Б").expect("str16");
        stream
            .write_all(&response_frame(
                Method::ListContacts as u16,
                header.request_id,
                Status::Ok,
                &answer,
            ))
            .await
            .expect("ответ отправлен");

        assert_eq!(
            reply.await.expect("задача не паникует"),
            Ok(OkReply::Contacts(vec![Contact {
                cid: Cid(5),
                name: "Аня Б".into(),
                online: true,
                blocked: false,
            }]))
        );
        let events = events.await;
        assert_eq!(events.front(), Some(&LinkEvent::Resync), "коннект = ресинк");
        cleanup(&path);
    }

    #[tokio::test]
    async fn send_message_binds_core_id_and_maps_kernel_error() {
        let path = temp_socket("send");
        cleanup(&path);
        let listener = UnixListener::bind(&path).expect("сокет забинден");
        let link = Arc::new(UdsLink::with_config(
            path.clone(),
            Duration::from_millis(10),
            false,
        ));
        let _events = CoreLink::events(Arc::clone(&link));

        let (mut stream, _) = listener.accept().await.expect("клиент подключился");
        let task = {
            let link = Arc::clone(&link);
            tokio::spawn(async move {
                CoreLink::execute(
                    &*link,
                    Request::SendMsg {
                        cid: Cid(3),
                        text: "привет".into(),
                    },
                )
                .await
            })
        };

        let (header, payload) = read_frame(&mut stream).await;
        assert_eq!(header.id, Method::SendMessage as u16);
        let mut r = api::Reader::new(&payload);
        assert_eq!(r.read_u32(), Ok(3));
        assert_eq!(
            r.read_str16().map(|b| std::str::from_utf8(b).unwrap()),
            Ok("привет")
        );

        let mut answer = Vec::new();
        answer.extend_from_slice(&[0xAB; 16]);
        api::put::u8(&mut answer, 1);
        stream
            .write_all(&response_frame(
                Method::SendMessage as u16,
                header.request_id,
                Status::Ok,
                &answer,
            ))
            .await
            .expect("ответ отправлен");
        assert_eq!(
            task.await.expect("задача не паникует"),
            Ok(OkReply::Sent {
                msg_id: CoreMsgId([0xAB; 16]),
                queued: true
            }),
            "queued=1 — сообщение в outbox ядра"
        );

        // Второй запрос — ядро отвечает машинным кодом.
        let task = {
            let link = Arc::clone(&link);
            tokio::spawn(async move {
                CoreLink::execute(
                    &*link,
                    Request::SendMsg {
                        cid: Cid(3),
                        text: "ещё".into(),
                    },
                )
                .await
            })
        };
        let (header, _) = read_frame(&mut stream).await;
        stream
            .write_all(&response_frame(
                Method::SendMessage as u16,
                header.request_id,
                Status::ContactBlocked,
                &[],
            ))
            .await
            .expect("ответ отправлен");
        let reply = task.await.expect("задача не паникует");
        assert_eq!(reply, Err(LinkError::Kernel(Status::ContactBlocked)));
        assert_eq!(
            reply.unwrap_err().user_text(),
            "Контакт заблокирован. Разблокируйте, чтобы писать."
        );
        cleanup(&path);
    }

    #[tokio::test]
    async fn kernel_events_reach_the_stream() {
        let path = temp_socket("events");
        cleanup(&path);
        let listener = UnixListener::bind(&path).expect("сокет забинден");
        let link = Arc::new(UdsLink::with_config(
            path.clone(),
            Duration::from_millis(10),
            false,
        ));
        let events = take_events(CoreLink::events(Arc::clone(&link)), 3);

        let (mut stream, _) = listener.accept().await.expect("клиент подключился");

        let mut presence = Vec::new();
        api::put::u32(&mut presence, 7);
        api::put::u8(&mut presence, 1);
        api::put::i64(&mut presence, 42);
        stream
            .write_all(&event_frame(api::Event::Presence, &presence))
            .await
            .expect("событие");

        let mut received = Vec::new();
        api::put::u32(&mut received, 7);
        received.extend_from_slice(&[0x11; 16]);
        api::put::u64(&mut received, 1_700_000_000_000);
        api::put::str16(&mut received, "Аня").expect("str16");
        api::put::str16(&mut received, "привет, мир").expect("str16");
        stream
            .write_all(&event_frame(api::Event::MessageReceived, &received))
            .await
            .expect("событие");

        let events = events.await;
        assert_eq!(events[0], LinkEvent::Resync, "коннект = ресинк");
        assert_eq!(
            events[1],
            LinkEvent::Presence {
                cid: Cid(7),
                online: true,
                last_seen_ms: 42
            }
        );
        assert_eq!(
            events[2],
            LinkEvent::MessageReceived {
                cid: Cid(7),
                name: "Аня".into(),
                msg_id: CoreMsgId([0x11; 16]),
                ts_ms: 1_700_000_000_000,
                text: "привет, мир".into(),
            }
        );
        cleanup(&path);
    }

    #[tokio::test]
    async fn disconnect_drains_pending_and_resyncs() {
        let path = temp_socket("reconn");
        cleanup(&path);
        let listener = UnixListener::bind(&path).expect("сокет забинден");
        let link = Arc::new(UdsLink::with_config(
            path.clone(),
            Duration::from_millis(10),
            false,
        ));
        let events = take_events(CoreLink::events(Arc::clone(&link)), 3);

        // Коннект 1: обычный обмен.
        let (mut stream, _) = listener.accept().await.expect("коннект 1");
        let task = {
            let link = Arc::clone(&link);
            tokio::spawn(async move { CoreLink::execute(&*link, Request::Contacts).await })
        };
        let (header, _) = read_frame(&mut stream).await;
        let mut answer = Vec::new();
        api::put::u32(&mut answer, 0);
        stream
            .write_all(&response_frame(
                Method::ListContacts as u16,
                header.request_id,
                Status::Ok,
                &answer,
            ))
            .await
            .expect("ответ");
        assert_eq!(
            task.await.expect("задача не паникует"),
            Ok(OkReply::Contacts(vec![]))
        );

        // Запрос в полёте — и ядро умирает, не ответив.
        let task2 = {
            let link = Arc::clone(&link);
            tokio::spawn(async move { CoreLink::execute(&*link, Request::Contacts).await })
        };
        let _ = read_frame(&mut stream).await; // запрос дошёл по коннекту 1
        drop(stream); // ...ответа не будет
        assert_eq!(
            task2.await.expect("задача не паникует"),
            Err(LinkError::Disconnected),
            "зависший запрос получает Disconnected, а не висит вечно"
        );

        // Реконнект: Resync, дальше запрос ходит по новому соединению.
        let (mut stream2, _) = listener.accept().await.expect("коннект 2 — ресинк");
        let task3 = {
            let link = Arc::clone(&link);
            tokio::spawn(async move { CoreLink::execute(&*link, Request::Contacts).await })
        };
        let (header, _) = read_frame(&mut stream2).await;
        let mut answer = Vec::new();
        api::put::u32(&mut answer, 1);
        api::put::u32(&mut answer, 9);
        api::put::u8(&mut answer, 0);
        api::put::i64(&mut answer, 0);
        api::put::str16(&mut answer, "Борис").expect("str16");
        stream2
            .write_all(&response_frame(
                Method::ListContacts as u16,
                header.request_id,
                Status::Ok,
                &answer,
            ))
            .await
            .expect("ответ");
        assert_eq!(
            task3.await.expect("задача не паникует"),
            Ok(OkReply::Contacts(vec![Contact {
                cid: Cid(9),
                name: "Борис".into(),
                online: false,
                blocked: false,
            }]))
        );

        let events = events.await;
        assert_eq!(events[0], LinkEvent::Resync, "первый коннект");
        assert_eq!(events[1], LinkEvent::Down, "обрыв первого");
        assert_eq!(events[2], LinkEvent::Resync, "ресинк после реконнекта");
        cleanup(&path);
    }

    #[tokio::test]
    async fn accept_invite_block_unblock_payloads_on_wire() {
        let path = temp_socket("meet");
        cleanup(&path);
        let listener = UnixListener::bind(&path).expect("сокет забинден");
        let link = Arc::new(UdsLink::with_config(
            path.clone(),
            Duration::from_millis(10),
            false,
        ));
        let _events = CoreLink::events(Arc::clone(&link));
        let (mut stream, _) = listener.accept().await.expect("коннект");

        // ACCEPT_INVITE: code str16 + my_name str16 (пустое — ядро подставит «друг»).
        let task = {
            let link = Arc::clone(&link);
            tokio::spawn(async move {
                CoreLink::execute(
                    &*link,
                    Request::Add {
                        code: "shopot://i/AbCdEf".into(),
                    },
                )
                .await
            })
        };
        let (header, payload) = read_frame(&mut stream).await;
        assert_eq!(header.id, Method::AcceptInvite as u16);
        let mut r = api::Reader::new(&payload);
        assert_eq!(
            r.read_str16().map(|b| std::str::from_utf8(b).unwrap()),
            Ok("shopot://i/AbCdEf")
        );
        assert_eq!(
            r.read_str16().map(|b| std::str::from_utf8(b).unwrap()),
            Ok(""),
            "my_name пустой: имя задаёт держатель кода у себя"
        );
        assert!(!r.has_more(), "строгий разбор payload");
        let mut answer = Vec::new();
        api::put::u32(&mut answer, 12);
        stream
            .write_all(&response_frame(
                Method::AcceptInvite as u16,
                header.request_id,
                Status::Ok,
                &answer,
            ))
            .await
            .expect("ответ");
        assert_eq!(
            task.await.expect("задача не паникует"),
            Ok(OkReply::Added { cid: Cid(12) })
        );

        // BLOCK_CONTACT: только cid u32; ответ пуст.
        for (method, request) in [
            (Method::BlockContact, Request::Block { cid: Cid(12) }),
            (Method::UnblockContact, Request::Unblock { cid: Cid(12) }),
        ] {
            let task = {
                let link = Arc::clone(&link);
                let request = request.clone();
                tokio::spawn(async move { CoreLink::execute(&*link, request).await })
            };
            let (header, payload) = read_frame(&mut stream).await;
            assert_eq!(header.id, method as u16);
            assert_eq!(payload, 12u32.to_le_bytes(), "payload = cid u32");
            stream
                .write_all(&response_frame(
                    method as u16,
                    header.request_id,
                    Status::Ok,
                    &[],
                ))
                .await
                .expect("ответ");
            assert_eq!(task.await.expect("задача не паникует"), Ok(OkReply::Done));
        }
        cleanup(&path);
    }

    #[tokio::test]
    async fn responses_route_by_request_id_even_out_of_order() {
        let path = temp_socket("ooo");
        cleanup(&path);
        let listener = UnixListener::bind(&path).expect("сокет забинден");
        let link = Arc::new(UdsLink::with_config(
            path.clone(),
            Duration::from_millis(10),
            false,
        ));
        let _events = CoreLink::events(Arc::clone(&link));

        let (mut stream, _) = listener.accept().await.expect("коннект");

        // Два запроса параллельно; ответы ядро шлёт в обратном порядке.
        let first = {
            let link = Arc::clone(&link);
            tokio::spawn(async move { CoreLink::execute(&*link, Request::Contacts).await })
        };
        let second = {
            let link = Arc::clone(&link);
            tokio::spawn(async move {
                CoreLink::execute(
                    &*link,
                    Request::Invite {
                        relay: false,
                        addr: String::new(),
                    },
                )
                .await
            })
        };

        let (h1, _) = read_frame(&mut stream).await;
        let (h2, _) = read_frame(&mut stream).await;

        let mut code = Vec::new();
        api::put::str16(&mut code, "shopot1.def").expect("str16");
        // Сначала ответ на ВТОРОЙ (h2), потом на ПЕРВЫЙ (h1).
        stream
            .write_all(&response_frame(h2.id, h2.request_id, Status::Ok, &code))
            .await
            .expect("ответ 2");
        let mut empty = Vec::new();
        api::put::u32(&mut empty, 0); // LIST_CONTACTS: count = 0
        stream
            .write_all(&response_frame(h1.id, h1.request_id, Status::Ok, &empty))
            .await
            .expect("ответ 1");

        assert_eq!(
            second.await.expect("задача не паникует"),
            Ok(OkReply::Code("shopot1.def".into()))
        );
        assert_eq!(
            first.await.expect("задача не паникует"),
            Ok(OkReply::Contacts(vec![]))
        );
        cleanup(&path);
    }
}
