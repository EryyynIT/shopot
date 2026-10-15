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

/// Процесс без дома (релей): Drop убивает даже при panic в тесте.
struct Proc(Child);

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
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

/// WAN-путь через relay: код с адресом релея, pairing по marker-у,
/// WAN-путь напрямую по адресу: код с host:port держателя, ядро
/// дозванивается само (CORE-API.md §4.2). Тот же сценарий, что у двух
/// разработчиков в разных городах с пробросом порта на роутере одного из них.
#[tokio::test(flavor = "multi_thread")]
async fn addr_invite_message_two_daemons() {
    let Ok(bin) = std::env::var("SHOPOT_BIN") else {
        eprintln!("SHOPOT_BIN не задан — e2e против демона пропущен");
        return;
    };

    // A пишет stderr в файл: ядро логирует выбранный порт слушателя.
    let home_a = std::env::temp_dir().join(format!("shopot-e2e-addr-a-{}", std::process::id()));
    std::fs::create_dir_all(&home_a).expect("дом A создан");
    let init = Command::new(&bin)
        .arg("init")
        .env("HOME", &home_a)
        .output()
        .expect("init A выполнен");
    assert!(init.status.success(), "init A не удался");
    let log_path = home_a.join("serve.log");
    let log = std::fs::File::create(&log_path).expect("лог A создан");
    let child_a = Command::new(&bin)
        .arg("serve")
        .env("HOME", &home_a)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::from(log))
        .spawn()
        .expect("демон A запущен");
    let a = Daemon(child_a, home_a);

    let b = spawn_daemon("addr-b");
    let sock_a = wait_socket(&a.1).await;
    let sock_b = wait_socket(&b.1).await;

    // Порт A из лога: «авто-режим: слушаю 0.0.0.0:{port}».
    let deadline = Instant::now() + Duration::from_secs(10);
    let port_a = loop {
        let text = std::fs::read_to_string(&log_path).unwrap_or_default();
        if let Some(pos) = text.find("слушаю 0.0.0.0:") {
            let digits: String = text[pos + "слушаю 0.0.0.0:".len()..]
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if let Ok(port) = digits.parse::<u16>() {
                break port;
            }
        }
        assert!(
            Instant::now() < deadline,
            "порт A не появился в логе за 10 с"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    };

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

    wait_event(&mut events_a, Duration::from_secs(10), |e| {
        matches!(e, LinkEvent::Resync)
    })
    .await;
    wait_event(&mut events_b, Duration::from_secs(10), |e| {
        matches!(e, LinkEvent::Resync)
    })
    .await;

    // A выдаёт код со своим адресом: B дозвонится напрямую, минуя LAN-поиск.
    let code = match CoreLink::execute(
        &*link_a,
        Request::Invite {
            relay: false,
            addr: format!("127.0.0.1:{port_a}"),
        },
    )
    .await
    {
        Ok(OkReply::Code(code)) => code,
        other => panic!("CREATE_INVITE с адресом не удался: {other:?}"),
    };

    let cid_b = match CoreLink::execute(&*link_b, Request::Add { code }).await {
        Ok(OkReply::Added { cid }) => cid,
        other => panic!("ACCEPT_INVITE по адресному коду не удался: {other:?}"),
    };

    wait_event(&mut events_a, Duration::from_secs(20), |e| {
        matches!(e, LinkEvent::ContactJoined { .. })
    })
    .await;
    wait_event(&mut events_b, Duration::from_secs(20), |e| {
        matches!(e, LinkEvent::Presence { online: true, .. })
    })
    .await;

    // B пишет A: сообщение доходит напрямую, ACK возвращается.
    let reply = CoreLink::execute(
        &*link_b,
        Request::SendMsg {
            cid: cid_b,
            text: "привет по адресному коду".into(),
        },
    )
    .await;
    let Ok(OkReply::Sent { msg_id, queued }) = reply else {
        panic!("SEND_MESSAGE по адресному коду не удался: {reply:?}");
    };
    assert!(!queued, "при живой сессии очередь не нужна");

    wait_event(&mut events_a, Duration::from_secs(20), |e| {
        matches!(e, LinkEvent::MessageReceived { .. })
    })
    .await;
    wait_event(
        &mut events_b,
        Duration::from_secs(20),
        |e| matches!(e, LinkEvent::MessageDelivered { msg_id: m, .. } if *m == msg_id),
    )
    .await;
}

/// S6: критерий приёмки «перезапуск GUI и демона — лента восстанавливается».
/// Переписка двух демонов, kill+restart демона A, GET_HISTORY отдаёт
/// переписку из зашифрованного лога с теми же msg_id и флагами.
#[tokio::test(flavor = "multi_thread")]
async fn history_survives_daemon_restart() {
    let Ok(bin) = std::env::var("SHOPOT_BIN") else {
        eprintln!("SHOPOT_BIN не задан — e2e против демона пропущен");
        return;
    };

    let mut a = spawn_daemon("hist-a");
    let b = spawn_daemon("hist-b");
    let sock_a = wait_socket(&a.1).await;
    let sock_b = wait_socket(&b.1).await;

    let link_a = std::sync::Arc::new(UdsLink::with_config(
        sock_a.clone(),
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
    wait_event(&mut events_a, Duration::from_secs(10), |e| {
        matches!(e, LinkEvent::Resync)
    })
    .await;
    wait_event(&mut events_b, Duration::from_secs(10), |e| {
        matches!(e, LinkEvent::Resync)
    })
    .await;

    // Знакомство: A выдаёт LAN-код, B гасит.
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
    let cid_b = match CoreLink::execute(&*link_b, Request::Add { code }).await {
        Ok(OkReply::Added { cid }) => cid,
        other => panic!("ACCEPT_INVITE не удался: {other:?}"),
    };
    let joined = wait_event(&mut events_a, Duration::from_secs(20), |e| {
        matches!(e, LinkEvent::ContactJoined { .. })
    })
    .await;
    let LinkEvent::ContactJoined { cid: cid_a, .. } = joined else {
        unreachable!()
    };
    wait_event(&mut events_b, Duration::from_secs(20), |e| {
        matches!(e, LinkEvent::Presence { online: true, .. })
    })
    .await;

    // Переписка: B пишет дважды, A отвечает.
    for text in ["первое до перезапуска", "второе до перезапуска"]
    {
        let reply = CoreLink::execute(
            &*link_b,
            Request::SendMsg {
                cid: cid_b,
                text: text.into(),
            },
        )
        .await;
        assert!(matches!(reply, Ok(OkReply::Sent { .. })), "{reply:?}");
        wait_event(
            &mut events_a,
            Duration::from_secs(20),
            |e| matches!(e, LinkEvent::MessageReceived { text: t, .. } if t == text),
        )
        .await;
    }
    let reply = CoreLink::execute(
        &*link_a,
        Request::SendMsg {
            cid: cid_a,
            text: "ответ A".into(),
        },
    )
    .await;
    assert!(matches!(reply, Ok(OkReply::Sent { .. })), "{reply:?}");
    wait_event(
        &mut events_b,
        Duration::from_secs(20),
        |e| matches!(e, LinkEvent::MessageReceived { text: t, .. } if t == "ответ A"),
    )
    .await;

    // Перезапуск демона A: тот же $HOME, свежий serve, свежий линк.
    // Drop у Daemon добьёт уже мёртвый child — повторный kill безвреден.
    let _ = a.0.kill();
    let _ = a.0.wait();
    let restarted = Proc(
        Command::new(&bin)
            .arg("serve")
            .env("HOME", &a.1)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("демон A перезапущен"),
    );
    let link_a2 = std::sync::Arc::new(UdsLink::with_config(
        sock_a,
        Duration::from_millis(100),
        false,
    ));
    let events_a2 = CoreLink::events(std::sync::Arc::clone(&link_a2));
    let mut events_a2 = Box::pin(events_a2);
    wait_event(&mut events_a2, Duration::from_secs(15), |e| {
        matches!(e, LinkEvent::Resync)
    })
    .await;

    // История у A: обе реплики B и свой ответ, порядок старые → новые.
    let reply = CoreLink::execute(
        &*link_a2,
        Request::History {
            cid: cid_a,
            limit: 50,
        },
    )
    .await;
    let Ok(OkReply::History { total, records }) = reply else {
        panic!("GET_HISTORY после перезапуска не удался: {reply:?}");
    };
    assert_eq!(total, 3, "три записи в логе демона A");
    let bodies: Vec<&str> = records.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(
        bodies,
        ["первое до перезапуска", "второе до перезапуска", "ответ A"],
        "порядок старые → новые"
    );
    assert!(
        !records[0].mine && !records[1].mine && records[2].mine,
        "флаги mine согласованы с направлением"
    );

    // История у B: его лог зеркалит переписку.
    let reply = CoreLink::execute(
        &*link_b,
        Request::History {
            cid: cid_b,
            limit: 50,
        },
    )
    .await;
    let Ok(OkReply::History { total, records }) = reply else {
        panic!("GET_HISTORY у B не удался: {reply:?}");
    };
    assert_eq!(total, 3, "у B три записи: {records:?}");
    assert!(
        records[0].mine && records[1].mine && !records[2].mine,
        "у B свои две реплики, чужой ответ последним: {records:?}"
    );
    assert!(
        records.iter().all(|r| !r.mine || r.delivered),
        "свои сообщения подтверждены ACK"
    );

    drop(restarted);
}

/// WAN-путь через relay: код с адресом релея, pairing по marker-у.
/// НЕСТАБИЛЕН: pairing иногда не завершается, первая отправка через свежий
/// relay-сеанс может зависнуть (CORE_FEEDBACK.md §5). Замер на ядре
/// 3334fb8 с tail-фиксом: 3 из 8 прогонов (~37%), на acffb0e — ~17%.
/// Ручной диагностический прогон: --ignored, не гейт CI.
#[ignore = "relay-путь ядра acffb0e нестабилен — CORE_FEEDBACK.md §5"]
#[tokio::test(flavor = "multi_thread")]
async fn relay_invite_message_two_daemons() {
    let Ok(bin) = std::env::var("SHOPOT_BIN") else {
        eprintln!("SHOPOT_BIN не задан — e2e против демона пропущен");
        return;
    };

    // Релей не требует личности и слушает TCP: порт вне диапазона демонов.
    let addr = "127.0.0.1:17890";
    let relay = Proc(
        Command::new(&bin)
            .arg("relay")
            .arg(addr)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("релей запущен"),
    );

    // Ждём, пока релей начнёт принимать соединения.
    let probe = std::net::SocketAddr::from(([127, 0, 0, 1], 17890));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if std::net::TcpStream::connect_timeout(&probe, Duration::from_millis(200)).is_ok() {
            break;
        }
        assert!(Instant::now() < deadline, "релей не поднял {addr} за 10 с");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let a = spawn_daemon("relay-a");
    let b = spawn_daemon("relay-b");
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

    wait_event(&mut events_a, Duration::from_secs(10), |e| {
        matches!(e, LinkEvent::Resync)
    })
    .await;
    wait_event(&mut events_b, Duration::from_secs(10), |e| {
        matches!(e, LinkEvent::Resync)
    })
    .await;

    // A выдаёт relay-код: соединение пойдёт через релей, IP A скрыт.
    let code = match CoreLink::execute(
        &*link_a,
        Request::Invite {
            relay: true,
            addr: addr.to_owned(),
        },
    )
    .await
    {
        Ok(OkReply::Code(code)) => code,
        other => panic!("CREATE_INVITE relay не удался: {other:?}"),
    };

    // B гасит код; ядро дозванивается через релей само.
    let cid_b = match CoreLink::execute(&*link_b, Request::Add { code }).await {
        Ok(OkReply::Added { cid }) => cid,
        other => panic!("ACCEPT_INVITE по relay-коду не удался: {other:?}"),
    };

    // Pairing по marker-у: сессия поднимается через релей.
    wait_event(&mut events_a, Duration::from_secs(30), |e| {
        matches!(e, LinkEvent::ContactJoined { .. })
    })
    .await;
    wait_event(&mut events_b, Duration::from_secs(30), |e| {
        matches!(e, LinkEvent::Presence { online: true, .. })
    })
    .await;

    // B пишет A: сообщение уходит через релей, ACK возвращается.
    let reply = CoreLink::execute(
        &*link_b,
        Request::SendMsg {
            cid: cid_b,
            text: "привет через релей".into(),
        },
    )
    .await;
    let Ok(OkReply::Sent { msg_id, queued }) = reply else {
        panic!("SEND_MESSAGE через релей не удался: {reply:?}");
    };
    assert!(!queued, "через релей сессия живая, очередь не нужна");

    wait_event(&mut events_a, Duration::from_secs(20), |e| {
        matches!(e, LinkEvent::MessageReceived { .. })
    })
    .await;
    wait_event(
        &mut events_b,
        Duration::from_secs(20),
        |e| matches!(e, LinkEvent::MessageDelivered { msg_id: m, .. } if *m == msg_id),
    )
    .await;

    drop(relay);
}
