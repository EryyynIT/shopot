//! UI-тесты на iced_test: headless-эмуляция всего приложения
//! (подписки и задачи — как вживую). Сцена — детерминированная.
//!
//! Все интерактивные сценарии выполняются в одном #[test]: эмуляторы
//! делят общий глобальный tokio-runtime iced, и при параллельном
//! запуске задачи одного эмулятора мешают доставке задач другого.

use iced::futures::channel::mpsc;
use iced_test::emulator::{self, Emulator, Mode};
use iced_test::instruction::{Expectation, Interaction, Key, Keyboard, Mouse, Target};
use iced_test::Instruction;
use shopot_gui::app::{self, Link};
use shopot_gui::core_link::mock::Scene;
use shopot_gui::model::cid::Cid;
use shopot_gui::model::msg::SendState;
use std::time::{Duration, Instant};

type Event<P> = emulator::Event<P>;

/// Качает события эмулятора `ms` миллисекунд реального времени.
fn pump<P: iced_test::program::Program + 'static>(
    emu: &mut Emulator<P>,
    program: &P,
    receiver: &mut mpsc::Receiver<Event<P>>,
    ms: u64,
) {
    let deadline = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < deadline {
        if let Ok(Event::Action(action)) = receiver.try_recv() {
            emu.perform(program, action);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Сливает всё, что уже пришло; возвращает число событий.
fn drain<P: iced_test::program::Program + 'static>(
    emu: &mut Emulator<P>,
    program: &P,
    receiver: &mut mpsc::Receiver<Event<P>>,
) -> usize {
    let mut seen = 0;
    while let Ok(event) = receiver.try_recv() {
        seen += 1;
        match event {
            Event::Action(action) => emu.perform(program, action),
            Event::Ready | Event::Failed(_) => {}
        }
    }
    seen
}

/// Ждёт «тихого периода»: окно, за которое не пришло ни одного события.
/// Гарантирует, что у всех задач, порождённых прошлыми действиями,
/// завершены и продолжения — их Ready/Failed не подделают результат
/// следующей инструкции.
fn wait_quiet<P: iced_test::program::Program + 'static>(
    emu: &mut Emulator<P>,
    program: &P,
    receiver: &mut mpsc::Receiver<Event<P>>,
    max_ms: u64,
) {
    let begin = Instant::now();
    let deadline = begin + Duration::from_millis(max_ms);
    let settled = begin + Duration::from_millis(200);
    while Instant::now() < deadline {
        let seen = drain(emu, program, receiver);
        // Тишина считается честной только после того, как все задачи
        // загрузки успели проявиться: в Immediate-режиме Ready от boot
        // приходит в канал с запаздыванием и без этого окна подделывает
        // результат первой же инструкции.
        if seen == 0 && Instant::now() >= settled {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Выполняет инструкцию и дожидается её результата на канале.
fn run_instruction<P: iced_test::program::Program + 'static>(
    emu: &mut Emulator<P>,
    program: &P,
    receiver: &mut mpsc::Receiver<Event<P>>,
    instruction: Instruction,
) {
    let label = format!("{instruction:?}");
    wait_quiet(emu, program, receiver, 3000);
    emu.run(program, instruction);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match receiver.try_recv() {
            Ok(Event::Action(action)) => emu.perform(program, action),
            Ok(Event::Ready) => return,
            Ok(Event::Failed(_)) => panic!("инструкция не выполнена: {label}"),
            Err(_) => {}
        }
        if Instant::now() > deadline {
            panic!("инструкция не завершилась за 5 с: {label}");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn click(text: &str) -> [Instruction; 2] {
    [
        Instruction::Interact(Interaction::Mouse(Mouse::Press {
            button: iced::mouse::Button::Left,
            target: Some(Target::Text(text.to_owned())),
        })),
        Instruction::Interact(Interaction::Mouse(Mouse::Release {
            button: iced::mouse::Button::Left,
            target: Some(Target::Text(text.to_owned())),
        })),
    ]
}

fn right_click(text: &str) -> [Instruction; 2] {
    [
        Instruction::Interact(Interaction::Mouse(Mouse::Press {
            button: iced::mouse::Button::Right,
            target: Some(Target::Text(text.to_owned())),
        })),
        Instruction::Interact(Interaction::Mouse(Mouse::Release {
            button: iced::mouse::Button::Right,
            target: Some(Target::Text(text.to_owned())),
        })),
    ]
}

#[test]
fn smoke_scenarios() {
    contacts_reply_populates_sidebar();
    incoming_message_counts_unread();
    send_flow_shows_delivered_and_autoreply();
    meet_scenarios();
}

/// Знакомство (S2): инвайт-диалог и блокировка через меню.
fn meet_scenarios() {
    invite_dialog_cycles_issue_and_add();
    invite_with_address_and_relay();
    right_click_menu_blocks_and_opens_card();
    escape_closes_overlay();
}

/// Код с адресом и relay: ядро дозванивается само, peer IP не видит
/// (CORE-API.md §4.2). Тот же путь, что и у двух разработчиков в разных
/// городах через релей на VPS.
fn invite_with_address_and_relay() {
    let program = app::program(Link::new(Scene::smoke()));
    let (sender, mut receiver) = mpsc::channel(512);
    let mut emu = Emulator::new(
        sender,
        &program,
        Mode::Immediate,
        iced::Size::new(1000.0, 700.0),
    );
    run_instruction(
        &mut emu,
        &program,
        &mut receiver,
        Instruction::Expect(Expectation::Text("Аня".into())),
    );

    let [press, release] = click("Пригласить");
    run_instruction(&mut emu, &program, &mut receiver, press);
    run_instruction(&mut emu, &program, &mut receiver, release);
    // Форма ждёт, код не выдан.
    run_instruction(
        &mut emu,
        &program,
        &mut receiver,
        Instruction::Expect(Expectation::Text("Код появится после выдачи.".into())),
    );

    // Адрес и relay-галка.
    let [press, release] = click("host:7777 — необязательно");
    run_instruction(&mut emu, &program, &mut receiver, press);
    run_instruction(&mut emu, &program, &mut receiver, release);
    run_instruction(
        &mut emu,
        &program,
        &mut receiver,
        Instruction::Interact(Interaction::Keyboard(Keyboard::Typewrite(
            "192.168.1.5:7777".into(),
        ))),
    );
    let [press, release] = click("через relay — собеседник не увидит ваш IP");
    run_instruction(&mut emu, &program, &mut receiver, press);
    run_instruction(&mut emu, &program, &mut receiver, release);

    // Выдача: код отражает relay-режим заглушки.
    let [press, release] = click("Выдать код");
    run_instruction(&mut emu, &program, &mut receiver, press);
    run_instruction(&mut emu, &program, &mut receiver, release);
    run_instruction(
        &mut emu,
        &program,
        &mut receiver,
        Instruction::Expect(Expectation::Text("Скопировать код".into())),
    );

    let (state, _) = emu.into_state();
    // relay-код не кэшируется как LAN: следующее открытие снова форма.
    assert!(
        state.invite_code.is_none(),
        "relay-код не должен попадать в LAN-кэш"
    );
    // На экране эллипсис случайного кода, поэтому режим сверяем по состоянию.
    if let shopot_gui::ui::Overlay::Invite(view) = &state.overlay {
        assert!(view.relay, "галка relay в форме выданного кода");
        let code = view.code.as_deref().expect("код выдан");
        assert!(
            code.starts_with("shopot1r."),
            "код relay-режима от заглушки: {code}"
        );
        assert!(
            code.ends_with("@192.168.1.5:7777"),
            "адрес попал в код: {code}"
        );
    } else {
        panic!("диалог приглашения должен остаться открытым");
    }
}

/// contacts → сайдбар populated: имя, статус «в сети», счётчик сессий.
fn contacts_reply_populates_sidebar() {
    let program = app::program(Link::new(Scene::smoke()));
    let (sender, mut receiver) = mpsc::channel(512);
    let mut emu = Emulator::new(
        sender,
        &program,
        Mode::Immediate,
        iced::Size::new(1000.0, 700.0),
    );
    run_instruction(
        &mut emu,
        &program,
        &mut receiver,
        Instruction::Expect(Expectation::Text("Аня".into())),
    );

    let (state, _) = emu.into_state();
    let contact = state.store.contact(Cid(1)).expect("контакт после contacts");
    assert_eq!(contact.name, "Аня");
    assert!(contact.online);
    assert_eq!(state.store.online_count(), 1);
}

/// Входящее при закрытом чате даёт непрочитанность ровно 1.
fn incoming_message_counts_unread() {
    let program = app::program(Link::new(Scene::smoke()));
    let (sender, mut receiver) = mpsc::channel(512);
    let mut emu = Emulator::new(
        sender,
        &program,
        Mode::Immediate,
        iced::Size::new(1000.0, 700.0),
    );
    pump(&mut emu, &program, &mut receiver, 1000);

    let (state, _) = emu.into_state();
    let thread = state.store.thread(Cid(1)).expect("Аня пришла из contacts");
    assert_eq!(
        thread.unread, 1,
        "входящее «пинг» вне открытого чата — бейдж 1"
    );
}

/// Отправка: ACK ядра показывает «доставлено», затем автоответ.
fn send_flow_shows_delivered_and_autoreply() {
    let program = app::program(Link::new(Scene::smoke()));
    let (sender, mut receiver) = mpsc::channel(512);
    let mut emu = Emulator::new(
        sender,
        &program,
        Mode::Immediate,
        iced::Size::new(1000.0, 700.0),
    );
    pump(&mut emu, &program, &mut receiver, 1000);

    let [press, release] = click("Аня");
    run_instruction(&mut emu, &program, &mut receiver, press);
    run_instruction(&mut emu, &program, &mut receiver, release);

    // Открытие чата фокусирует композер — печатаем сразу.
    run_instruction(
        &mut emu,
        &program,
        &mut receiver,
        Instruction::Interact(Interaction::Keyboard(Keyboard::Typewrite("привет".into()))),
    );
    run_instruction(
        &mut emu,
        &program,
        &mut receiver,
        Instruction::Interact(Interaction::Keyboard(Keyboard::Type(Key::Enter))),
    );

    pump(&mut emu, &program, &mut receiver, 1500);

    // Лента показывает доставку (ACK ядра) и автоответ.
    run_instruction(
        &mut emu,
        &program,
        &mut receiver,
        Instruction::Expect(Expectation::Text("доставлено".into())),
    );
    run_instruction(
        &mut emu,
        &program,
        &mut receiver,
        Instruction::Expect(Expectation::Text("принято".into())),
    );

    let (state, _) = emu.into_state();
    let thread = state.store.thread(Cid(1)).expect("лента Ани");
    assert_eq!(thread.unread, 0, "чат открыт — непрочитанных нет");
    let sent = thread
        .msgs
        .iter()
        .find(|m| m.body == "привет")
        .expect("моё сообщение в ленте");
    assert_eq!(
        sent.send,
        SendState::Delivered,
        "заглушка шлёт квази-ACK — как реальное ядро"
    );
    assert!(thread.msgs.iter().any(|m| m.body == "пинг"));
    assert!(thread.msgs.iter().any(|m| m.body == "принято"));
}

/// Инвайт-диалог: код заглушки отрисовался, чужой код гасится —
/// контакт появляется в сайдбаре и открывается.
fn invite_dialog_cycles_issue_and_add() {
    let program = app::program(Link::new(Scene::smoke()));
    let (sender, mut receiver) = mpsc::channel(512);
    let mut emu = Emulator::new(
        sender,
        &program,
        Mode::Immediate,
        iced::Size::new(1000.0, 700.0),
    );
    run_instruction(
        &mut emu,
        &program,
        &mut receiver,
        Instruction::Expect(Expectation::Text("Аня".into())),
    );

    // Кнопка сайдбара открывает диалог.
    let [press, release] = click("Пригласить");
    run_instruction(&mut emu, &program, &mut receiver, press);
    run_instruction(&mut emu, &program, &mut receiver, release);

    // Код выдаётся по кнопке: сначала форма, потом ответ заглушки.
    let [press, release] = click("Выдать код");
    run_instruction(&mut emu, &program, &mut receiver, press);
    run_instruction(&mut emu, &program, &mut receiver, release);
    run_instruction(
        &mut emu,
        &program,
        &mut receiver,
        Instruction::Expect(Expectation::Text("Скопировать код".into())),
    );

    // Чужой код: клик по полю, ввод, «Добавить».
    let [press, release] = click("Вставьте код приглашения");
    run_instruction(&mut emu, &program, &mut receiver, press);
    run_instruction(&mut emu, &program, &mut receiver, release);
    run_instruction(
        &mut emu,
        &program,
        &mut receiver,
        Instruction::Interact(Interaction::Keyboard(Keyboard::Typewrite(
            "shopot1.abc@192.168.1.5".into(),
        ))),
    );
    let [press, release] = click("Добавить");
    run_instruction(&mut emu, &program, &mut receiver, press);
    run_instruction(&mut emu, &program, &mut receiver, release);
    pump(&mut emu, &program, &mut receiver, 1200);

    let (state, _) = emu.into_state();
    let guest = state
        .store
        .contacts()
        .find(|c| c.name == "Гость 1")
        .expect("гость появился в сайдбаре");
    assert_eq!(
        state.store.selected(),
        Some(guest.cid),
        "новый контакт открыт"
    );
    assert!(state.overlay_closed(), "диалог закрыт после добавления");
}

/// Правая кнопка на строке контакта — меню: блокировка, карточка, закрытие.
fn right_click_menu_blocks_and_opens_card() {
    let program = app::program(Link::new(Scene::smoke()));
    let (sender, mut receiver) = mpsc::channel(512);
    let mut emu = Emulator::new(
        sender,
        &program,
        Mode::Immediate,
        iced::Size::new(1000.0, 700.0),
    );
    run_instruction(
        &mut emu,
        &program,
        &mut receiver,
        Instruction::Expect(Expectation::Text("Аня".into())),
    );

    let [press, release] = right_click("Аня");
    run_instruction(&mut emu, &program, &mut receiver, press);
    run_instruction(&mut emu, &program, &mut receiver, release);
    run_instruction(
        &mut emu,
        &program,
        &mut receiver,
        Instruction::Expect(Expectation::Text("Заблокировать".into())),
    );

    let [press, release] = click("Заблокировать");
    run_instruction(&mut emu, &program, &mut receiver, press);
    run_instruction(&mut emu, &program, &mut receiver, release);
    pump(&mut emu, &program, &mut receiver, 500);

    // Меню снова: теперь «Разблокировать», открываем карточку.
    let [press, release] = right_click("Аня");
    run_instruction(&mut emu, &program, &mut receiver, press);
    run_instruction(&mut emu, &program, &mut receiver, release);
    run_instruction(
        &mut emu,
        &program,
        &mut receiver,
        Instruction::Expect(Expectation::Text("Разблокировать".into())),
    );
    let [press, release] = click("Информация");
    run_instruction(&mut emu, &program, &mut receiver, press);
    run_instruction(&mut emu, &program, &mut receiver, release);
    run_instruction(
        &mut emu,
        &program,
        &mut receiver,
        Instruction::Expect(Expectation::Text("заблокирован".into())),
    );

    // Разблок из карточки: карточка не закрывается, статус обновляется
    // в ней же (GUI_STYLE.md §2, честность статусов).
    let [press, release] = click("Разблокировать");
    run_instruction(&mut emu, &program, &mut receiver, press);
    run_instruction(&mut emu, &program, &mut receiver, release);
    pump(&mut emu, &program, &mut receiver, 500);
    run_instruction(
        &mut emu,
        &program,
        &mut receiver,
        Instruction::Expect(Expectation::Text("в сети".into())),
    );

    let [press, release] = click("Закрыть");
    run_instruction(&mut emu, &program, &mut receiver, press);
    run_instruction(&mut emu, &program, &mut receiver, release);
    pump(&mut emu, &program, &mut receiver, 300);

    let (state, _) = emu.into_state();
    assert!(
        !state.store.contact(Cid(1)).expect("Аня").blocked,
        "разблок через карточку дошёл до Store"
    );
}

/// Esc закрывает оверлей, пока композер в фокусе (GUI_UI.md §3).
/// Подписка клавиатуры в headless-эмуляторе событий не получает, поэтому
/// Esc проверяется через привязку композера; живой путь — подписка.
fn escape_closes_overlay() {
    let program = app::program(Link::new(Scene::smoke()));
    let (sender, mut receiver) = mpsc::channel(512);
    let mut emu = Emulator::new(
        sender,
        &program,
        Mode::Immediate,
        iced::Size::new(1000.0, 700.0),
    );
    run_instruction(
        &mut emu,
        &program,
        &mut receiver,
        Instruction::Expect(Expectation::Text("Аня".into())),
    );
    // Фокус композера: открыть чат.
    let [press, release] = click("Аня");
    run_instruction(&mut emu, &program, &mut receiver, press);
    run_instruction(&mut emu, &program, &mut receiver, release);

    let [press, release] = click("Пригласить");
    run_instruction(&mut emu, &program, &mut receiver, press);
    run_instruction(&mut emu, &program, &mut receiver, release);
    // Оверлей открыт: форма вместо мгновенного кода.
    run_instruction(
        &mut emu,
        &program,
        &mut receiver,
        Instruction::Expect(Expectation::Text("Код появится после выдачи.".into())),
    );

    run_instruction(
        &mut emu,
        &program,
        &mut receiver,
        Instruction::Interact(Interaction::Keyboard(Keyboard::Type(Key::Escape))),
    );
    pump(&mut emu, &program, &mut receiver, 300);

    let (state, _) = emu.into_state();
    assert!(state.overlay_closed(), "Esc закрыл инвайт-диалог");
}

/// Скриншот демо-сцены для ручной визуальной приёмки.
#[test]
fn screenshot_demo_scene() {
    let program = app::program(Link::new(Scene::demo()));
    let shot = iced_test::screenshot(
        &program,
        &iced::Theme::Dark,
        iced::Size::new(1000.0, 700.0),
        1.0,
        Duration::from_secs(2),
    );

    // RGBA → PNG через dev-зависимость png.
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, shot.size.width, shot.size.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("png header");
        writer
            .write_image_data(&shot.rgba)
            .expect("rgba кадр пишется в PNG");
    }
    // Путь читает README (docs/screenshot.png): тест перегенерирует ассет.
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/docs/screenshot.png");
    std::fs::write(path, png).expect("скриншот сохраняется");
}
