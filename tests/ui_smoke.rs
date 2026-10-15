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

#[test]
fn smoke_scenarios() {
    contacts_reply_populates_sidebar();
    incoming_message_counts_unread();
    send_flow_shows_sent_and_autoreply();
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

/// Отправка: «отправлено» в ленте, автоответ, бейдж сброшен открытием.
fn send_flow_shows_sent_and_autoreply() {
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

    // Лента показывает и статус отправки, и автоответ.
    run_instruction(
        &mut emu,
        &program,
        &mut receiver,
        Instruction::Expect(Expectation::Text("отправлено".into())),
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
    assert_eq!(sent.send, SendState::Sent);
    assert!(thread.msgs.iter().any(|m| m.body == "пинг"));
    assert!(thread.msgs.iter().any(|m| m.body == "принято"));
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
    std::fs::write("/tmp/shopot-s1-demo.png", png).expect("скриншот сохраняется");
}
