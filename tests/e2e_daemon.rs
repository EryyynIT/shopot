//! E2E против реального демона shopot-core: два $HOME, два процесса
//! `shopot serve`, UdsLink с обеих сторон — инвайт → сообщение → доставка.
//!
//! Гейт: запускается только при заданном SHOPOT_BIN (путь к бинарнику
//! ядра); в CI без него тест молча пропускается. Запуск локально:
//!   SHOPOT_BIN=core/zig/zig-out/bin/shopot-zig cargo test --test e2e_daemon

use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use shopot_gui::core_link::api::Status;
use shopot_gui::core_link::uds::UdsLink;
use shopot_gui::core_link::{CoreLink, LinkError, LinkEvent, OkReply, Request};

struct Daemon(Child, PathBuf);

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
        let _ = std::fs::remove_dir_all(&self.1);
    }
}

fn spawn_daemon(tag: &str) -> Daemon {
    let bin = std::env::var("SHOPOT_BIN").expect("SHOPOT_BIN задан (гейт теста)");
    let home = std::env::temp_dir().join(format!("shopot-e2e-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&home).expect("дом демона создан");
    // serve требует личность: у свежего $HOME сначала init.
    let init = Command::new(&bin)
        .arg("init")
        .env("HOME", &home)
        .output()
        .expect("init выполнен");
    assert!(
        init.status.success(),
        "init не удался: {}",
        String::from_utf8_lossy(&init.stderr)
    );
    let child = Command::new(&bin)
        .arg("serve")
        .env("HOME", &home)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("демон запущен");
    Daemon(child, home)
}

async fn wait_socket(home: &std::path::Path) -> PathBuf {
    let sock = home.join(".shopot/api.sock");
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if sock.exists() {
            return sock;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("демон не поднял {sock:?} за 15 с");
}

/// Ждать событие, удовлетворяющее предикат; остальное копится в буфер.
async fn wait_event(
    stream: &mut (impl futures_core::Stream<Item = LinkEvent> + Unpin),
    timeout: Duration,
    pred: impl Fn(&LinkEvent) -> bool,
) -> LinkEvent {
    let deadline = Instant::now() + timeout;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            panic!("событие не пришло за {timeout:?}");
        }
        match tokio::time::timeout(left, stream.next()).await {
            Ok(Some(event)) if pred(&event) => return event,
            Ok(Some(_)) => continue,
            Ok(None) => panic!("поток событий закрылся"),
            Err(_) => panic!("таймаут ожидания события"),
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn invite_message_delivery_two_daemons() {
    let Ok(bin) = std::env::var("SHOPOT_BIN") else {
        eprintln!("SHOPOT_BIN не задан — e2e против демона пропущен");
        return;
    };
    let _ = bin; // читается в spawn_daemon

    let a = spawn_daemon("a");
    let b = spawn_daemon("b");
    let sock_a = wait_socket(&a.1).await;
    let sock_b = wait_socket(&b.1).await;

    let link_a = std::sync::Arc::new(UdsLink::with_config(
        sock_a,
        Duration::from_millis(100),
        false,
    ));
    let link_b = std::sync::Arc::new(UdsLink::with_config(
        sock_b,
        Duration::from_millis(100),
        false,
    ));

    let events_a = CoreLink::events(std::sync::Arc::clone(&link_a));
    let mut events_a = Box::pin(events_a);
    let events_b = CoreLink::events(std::sync::Arc::clone(&link_b));
    let mut events_b = Box::pin(events_b);

    // Сигнал готовности: Resync = супервизор подключился к демону.
    wait_event(&mut events_a, Duration::from_secs(10), |e| {
        matches!(e, LinkEvent::Resync)
    })
    .await;
    wait_event(&mut events_b, Duration::from_secs(10), |e| {
        matches!(e, LinkEvent::Resync)
    })
    .await;

    // Оба демона живы: список контактов пуст.
    let reply = CoreLink::execute(&*link_a, Request::Contacts).await;
    assert!(
        matches!(&reply, Ok(OkReply::Contacts(contacts)) if contacts.is_empty()),
        "у свежего демона контактов нет: {reply:?}"
    );

    // A выдаёт инвайт-код, B гасит его — контакт появляется у обоих.
    let code = match CoreLink::execute(
        &*link_a,
        Request::Invite {
            relay: false,
            addr: String::new(),
        },
    )
    .await
    {
        Ok(OkReply::Code(code)) => code,
        other => panic!("CREATE_INVITE не удался: {other:?}"),
    };
    assert!(
        code.starts_with("shopot1.") || code.starts_with("shopot://i/"),
        "код: {code}"
    );

    let cid_b = match CoreLink::execute(&*link_b, Request::Add { code: code.clone() }).await {
        Ok(OkReply::Added { cid }) => cid,
        other => panic!("ACCEPT_INVITE не удался: {other:?}"),
    };

    // A видит, что держатель кода подключился.
    let joined = wait_event(&mut events_a, Duration::from_secs(20), |e| {
        matches!(e, LinkEvent::ContactJoined { .. })
    })
    .await;
    let LinkEvent::ContactJoined { cid: cid_a, name } = joined else {
        unreachable!()
    };

    // Сессия с B поднята (presence online) — иначе отправка уйдёт в outbox.
    wait_event(&mut events_b, Duration::from_secs(20), |e| {
        matches!(e, LinkEvent::Presence { online: true, .. })
    })
    .await;

    // B пишет A: сообщение доходит, ACK возвращается B.
    let reply = CoreLink::execute(
        &*link_b,
        Request::SendMsg {
            cid: cid_b,
            text: "привет из e2e".into(),
        },
    )
    .await;
    let Ok(OkReply::Sent { msg_id, queued }) = reply else {
        panic!("SEND_MESSAGE не удался: {reply:?}");
    };
    assert!(!queued, "при живой сессии очередь не нужна");

    let received = wait_event(&mut events_a, Duration::from_secs(20), |e| {
        matches!(e, LinkEvent::MessageReceived { .. })
    })
    .await;
    let LinkEvent::MessageReceived {
        cid,
        name: sender_name,
        text,
        msg_id: received_id,
        ..
    } = received
    else {
        unreachable!()
    };
    assert_eq!(cid, cid_a);
    assert_eq!(text, "привет из e2e");
    assert_eq!(received_id, msg_id, "msg_id ядра сквозной");
    assert_eq!(sender_name, name, "имя B согласовано с ContactJoined");

    // ACK доходит до отправителя: статус «доставлено».
    wait_event(
        &mut events_b,
        Duration::from_secs(20),
        |e| matches!(e, LinkEvent::MessageDelivered { msg_id: m, .. } if *m == msg_id),
    )
    .await;

    // S2: блок/разблок на реальном ядре. Заблокированному отправка
    // отклоняется CONTACT_BLOCKED, после разблокировки снова уходит.
    assert_eq!(
        CoreLink::execute(&*link_b, Request::Block { cid: cid_b }).await,
        Ok(OkReply::Done),
        "BLOCK_CONTACT"
    );
    let reply = CoreLink::execute(
        &*link_b,
        Request::SendMsg {
            cid: cid_b,
            text: "не уйдёт".into(),
        },
    )
    .await;
    assert_eq!(reply, Err(LinkError::Kernel(Status::ContactBlocked)));
    assert_eq!(
        CoreLink::execute(&*link_b, Request::Unblock { cid: cid_b }).await,
        Ok(OkReply::Done),
        "UNBLOCK_CONTACT"
    );
    let reply = CoreLink::execute(
        &*link_b,
        Request::SendMsg {
            cid: cid_b,
            text: "снова уходит".into(),
        },
    )
    .await;
    assert!(matches!(reply, Ok(OkReply::Sent { .. })), "{reply:?}");
}
