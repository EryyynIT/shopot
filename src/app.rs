//! Сшивка: CoreLink-поток → Message → Store → view.
//! Строится по зонам из GUI_ARCHITECTURE.md §1.

use std::sync::Arc;

use iced::{Subscription, Task};

use crate::core_link::mock::{MockLink, Scene};
use crate::core_link::{CoreLink, Request};
use crate::model::msg::MsgId;
use crate::ui::{self, LinkIn, Message};

fn update(state: &mut App, message: Message) -> Task<Message> {
    ui::update(state, message)
}

fn view(state: &App) -> iced::Element<'_, Message> {
    ui::view(state)
}

/// Канал с ядром. Появится реальный транспорт (S5) — замена одной строкой.
pub type Link = MockLink;

/// Стейт программы.
pub type App = ui::State<Link>;

pub fn run() -> iced::Result {
    program(Link::new(Scene::demo())).run()
}

/// Собирает Program приложения: один и тот же для окна и для iced_test.
pub fn program(
    link: Link,
) -> iced::application::Application<
    impl iced::Program<State = App, Message = Message, Theme = iced::Theme>,
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
struct Boot(Arc<Link>);

impl iced::application::BootFn<App, Message> for Boot {
    fn boot(&self) -> (App, Task<Message>) {
        boot(Arc::clone(&self.0))
    }
}

fn boot(link: Arc<Link>) -> (App, Task<Message>) {
    let state = ui::State::new(Arc::clone(&link));
    let task = link_task(link, Request::Contacts, None);
    (state, task)
}

/// Постоянный поток событий ядра; идентификатор подписки — сам линк.
fn subscription(state: &App) -> Subscription<Message> {
    Subscription::run_with(Arc::clone(&state.link), events_subscription)
}

fn events_subscription(link: &Arc<Link>) -> impl iced::futures::Stream<Item = Message> {
    use iced::futures::StreamExt;
    CoreLink::events(Arc::clone(link)).map(|event| Message::Link(LinkIn::Event(event)))
}

fn theme(_: &App) -> iced::Theme {
    iced::Theme::Dark
}

/// Разовый запрос ядру; ответ возвращается через Replied.
fn link_task(link: Arc<Link>, request: Request, tag: Option<MsgId>) -> Task<Message> {
    let reply_request = request.clone();
    Task::perform(link.execute(request), move |reply| {
        Message::Link(LinkIn::Replied {
            request: reply_request.clone(),
            reply,
            tag,
        })
    })
}
