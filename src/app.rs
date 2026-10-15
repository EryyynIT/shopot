//! Сшивка: CoreLink-поток → Message → Store → view.
//! Строится по зонам из GUI_ARCHITECTURE.md §1.

use std::sync::Arc;

use iced::{Subscription, Task};

use crate::core_link::mock::{MockLink, Scene};
use crate::core_link::uds::UdsLink;
use crate::core_link::{CoreLink, Request};
use crate::model::msg::MsgId;
use crate::ui::{self, LinkIn, MeetIn, Message};

fn update<L: CoreLink>(state: &mut App<L>, message: Message) -> Task<Message> {
    ui::update(state, message)
}

fn view<L: CoreLink>(state: &App<L>) -> iced::Element<'_, Message> {
    ui::view(state)
}

/// Канал демо-режима (для тестов и `--mock`); рабочий — [`UdsLink`].
pub type Link = MockLink;

/// Стейт программы.
pub type App<L> = ui::State<L>;

pub fn run() -> iced::Result {
    // Реальный канал — по умолчанию; `--mock` — демо-сцена без ядра.
    if std::env::args().any(|arg| arg == "--mock") {
        program(MockLink::new(Scene::demo())).run()
    } else {
        program(UdsLink::new()).run()
    }
}

/// Собирает Program приложения: один и тот же для окна и для iced_test.
pub fn program<L: CoreLink>(
    link: L,
) -> iced::application::Application<
    impl iced::Program<State = App<L>, Message = Message, Theme = iced::Theme>,
> {
    iced::application(Boot(Arc::new(link)), update, view)
        .title("shopot")
        .theme(theme)
        .window(iced::window::Settings {
            size: iced::Size::new(1000.0, 700.0),
            min_size: Some(iced::Size::new(800.0, 560.0)),
            ..Default::default()
        })
        .subscription(subscription)
}

/// Загрузчик приложения: держит линк для boot.
struct Boot<L>(Arc<L>);

impl<L: CoreLink> iced::application::BootFn<App<L>, Message> for Boot<L> {
    fn boot(&self) -> (App<L>, Task<Message>) {
        boot(Arc::clone(&self.0))
    }
}

fn boot<L: CoreLink>(link: Arc<L>) -> (App<L>, Task<Message>) {
    let state = ui::State::new(Arc::clone(&link));
    let task = link_task(link, Request::Contacts, None);
    (state, task)
}

/// Постоянный поток событий ядра + клавиатура (Esc закрывает оверлей).
fn subscription<L: CoreLink>(state: &App<L>) -> Subscription<Message> {
    Subscription::batch([
        Subscription::run_with(Arc::clone(&state.link), events_subscription),
        esc_subscription(),
    ])
}

fn events_subscription<L: CoreLink>(link: &Arc<L>) -> impl iced::futures::Stream<Item = Message> {
    use iced::futures::StreamExt;
    CoreLink::events(Arc::clone(link)).map(|event| Message::Link(LinkIn::Event(event)))
}

/// Esc закрывает оверлей (GUI_UI.md §3). `listen()` отдаёт только события,
/// которые не съел виджет: привязки композера не задеваются.
fn esc_subscription() -> Subscription<Message> {
    use iced::keyboard::key::Named;
    use iced::keyboard::{Event as KeyEvent, Key};
    iced::keyboard::listen().filter_map(|event| match event {
        KeyEvent::KeyPressed {
            key: Key::Named(Named::Escape),
            ..
        } => Some(Message::Meet(MeetIn::CloseOverlay)),
        _ => None,
    })
}

fn theme<L: CoreLink>(_: &App<L>) -> iced::Theme {
    iced::Theme::Dark
}

/// Разовый запрос ядру; ответ возвращается через Replied.
fn link_task<L: CoreLink>(link: Arc<L>, request: Request, tag: Option<MsgId>) -> Task<Message> {
    let reply_request = request.clone();
    Task::perform(link.execute(request), move |reply| {
        Message::Link(LinkIn::Replied {
            request: reply_request.clone(),
            reply,
            tag,
        })
    })
}
