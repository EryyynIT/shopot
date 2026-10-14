//! UI: Elm-архитектура — один State, update() как единственное место
//! мутаций, view() — чистая функция над Store (GUI_DEV.md §3).

pub mod palette;
pub mod widgets;

use std::sync::Arc;

use chrono::Datelike;
use iced::alignment::{Horizontal, Vertical};
use iced::keyboard::{self, key::Named};
use iced::widget::{column, container, lazy, operation, row, scrollable, space, text, text_editor};
use iced::{Element, Length, Padding, Task};

use crate::core_link::{now_ms, parse_contacts, CoreLink, LinkEvent, Reply, Request};
use crate::model::cid::Cid;
use crate::model::feed::FeedItem;
use crate::model::msg::MsgId;
use crate::model::store::Store;

/// Сообщения приложения: канал ядра и локальные действия чата.
#[derive(Debug, Clone)]
pub enum Message {
    Link(LinkIn),
    Chat(ChatIn),
}

/// Ответы и события core_link. Не протекают в виджеты: update пишет
/// их в Store, view читает Store.
#[derive(Debug, Clone)]
pub enum LinkIn {
    Event(LinkEvent),
    Replied {
        request: Request,
        reply: Reply,
        tag: Option<MsgId>,
    },
}

/// Локальные действия пользователя.
#[derive(Debug, Clone)]
pub enum ChatIn {
    Select(Cid),
    Send,
    Edit(text_editor::Action),
}

/// Весь стейт приложения; дженерик по каналу с ядром — ui не знает,
/// заглушка это или UDS-транспорт.
pub struct State<L: CoreLink> {
    pub link: Arc<L>,
    pub store: Store,
    /// Содержимое композера; текст черновика хранит Store.
    pub composer: text_editor::Content,
    pub feed_id: iced::widget::Id,
    pub composer_id: iced::widget::Id,
}

impl<L: CoreLink> State<L> {
    pub fn new(link: Arc<L>) -> Self {
        Self {
            link,
            store: Store::new(),
            composer: text_editor::Content::new(),
            feed_id: iced::widget::Id::unique(),
            composer_id: iced::widget::Id::unique(),
        }
    }
}

pub fn update<L: CoreLink>(state: &mut State<L>, message: Message) -> Task<Message> {
    match message {
        Message::Link(link) => on_link(state, link),
        Message::Chat(chat) => on_chat(state, chat),
    }
}

fn on_link<L: CoreLink>(state: &mut State<L>, link: LinkIn) -> Task<Message> {
    match link {
        LinkIn::Event(event) => match event {
            LinkEvent::IMsg {
                cid,
                name,
                msg_id,
                ts_ms,
                text,
            } => {
                let opened = state.store.selected() == Some(cid);
                state.store.push_incoming(cid, &name, msg_id, ts_ms, text);
                if opened {
                    scroll_feed(state)
                } else {
                    Task::none()
                }
            }
            LinkEvent::Presence { cid, online } => {
                state.store.set_presence(cid, online);
                Task::none()
            }
            LinkEvent::Sys { cid, text } => {
                state.store.push_sys(cid, now_ms(), text);
                Task::none()
            }
            // Заглушке не приходит; обработка появится с uds-транспортом.
            LinkEvent::Down => Task::none(),
        },
        LinkIn::Replied {
            request,
            reply,
            tag,
        } => match (request, reply) {
            (Request::Contacts, Reply::Ok(lines)) => {
                state.store.replace_contacts(parse_contacts(&lines));
                Task::none()
            }
            (Request::SendMsg { cid, .. }, Reply::Ok(_)) => {
                if let Some(tag) = tag {
                    state.store.mark_sent(cid, tag);
                }
                Task::none()
            }
            (Request::SendMsg { cid, .. }, Reply::Err(reason)) => {
                if let Some(tag) = tag {
                    state.store.mark_failed(cid, tag, reason);
                }
                Task::none()
            }
            // Остальные команды S1 не отправляет из UI.
            _ => Task::none(),
        },
    }
}

fn on_chat<L: CoreLink>(state: &mut State<L>, chat: ChatIn) -> Task<Message> {
    match chat {
        ChatIn::Select(cid) => {
            state.store.select(Some(cid));
            let draft = state.store.draft(cid).to_owned();
            state.composer = text_editor::Content::with_text(&draft);
            Task::batch([
                scroll_feed(state),
                operation::focus::<Message>(state.composer_id.clone()),
            ])
        }
        ChatIn::Edit(action) => {
            state.composer.perform(action);
            if let Some(cid) = state.store.selected() {
                let draft = state.composer.text();
                state.store.set_draft(cid, draft);
            }
            Task::none()
        }
        ChatIn::Send => send_current(state),
    }
}

fn send_current<L: CoreLink>(state: &mut State<L>) -> Task<Message> {
    let Some(cid) = state.store.selected() else {
        return Task::none();
    };
    let text = state.composer.text();
    if text.trim().is_empty() {
        return Task::none();
    }

    let msg_id = state.store.push_outgoing(cid, now_ms(), text.clone());
    state.composer = text_editor::Content::new();
    state.store.set_draft(cid, String::new());

    let request = Request::SendMsg { cid, text };
    let reply_request = request.clone();
    let link = Arc::clone(&state.link);
    Task::batch([
        Task::perform(link.execute(request), move |reply| {
            Message::Link(LinkIn::Replied {
                request: reply_request.clone(),
                reply,
                tag: Some(msg_id),
            })
        }),
        scroll_feed(state),
    ])
}

/// Лента всегда прокручивается к самому свежему сообщению.
fn scroll_feed<L: CoreLink>(state: &State<L>) -> Task<Message> {
    operation::snap_to_end::<Message>(state.feed_id.clone())
}

/// Вид: окно целиком (GUI_UI.md §1).
pub fn view<L: CoreLink>(state: &State<L>) -> Element<'_, Message> {
    container(
        column![
            title_bar(),
            row![sidebar(state), main_pane(state)].height(Length::Fill),
            status_bar(state),
        ]
        .width(Length::Fill)
        .height(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .style(palette::root)
    .into()
}

fn title_bar() -> Element<'static, Message> {
    container(
        row![
            text("shopot")
                .size(14)
                .font(palette::bold())
                .color(palette::ACCENT),
            space::Space::new().width(Length::Fill),
            text("режим: заглушка").size(11).color(palette::DIM),
        ]
        .align_y(Vertical::Center),
    )
    .width(Length::Fill)
    .height(Length::Fixed(32.0))
    .padding(Padding {
        top: 0.0,
        right: 12.0,
        bottom: 0.0,
        left: 12.0,
    })
    .style(palette::panel)
    .into()
}

fn sidebar<L: CoreLink>(state: &State<L>) -> Element<'_, Message> {
    let selected = state.store.selected();
    // Снимок строк + ключ ленивости: список перестраивается только при
    // изменении состава/статусов (GUI_DEV.md §3 — список чатов lazy).
    let rows: Vec<(Cid, String, bool, u32, bool)> = state
        .store
        .contacts()
        .map(|c| {
            let unread = state.store.thread(c.cid).map(|t| t.unread).unwrap_or(0);
            (
                c.cid,
                c.name.clone(),
                c.online,
                unread,
                selected == Some(c.cid),
            )
        })
        .collect();

    let list: Element<'static, Message> = if rows.is_empty() {
        container(
            text("Пока контактов нет. Создайте код приглашения или вставьте чужой.")
                .size(12)
                .color(palette::DIM),
        )
        .padding(Padding {
            top: 8.0,
            right: 4.0,
            bottom: 8.0,
            left: 4.0,
        })
        .into()
    } else {
        let key: Vec<(i64, u32, bool, bool)> = rows
            .iter()
            .map(|(cid, _, online, unread, chosen)| (cid.0, *unread, *online, *chosen))
            .collect();
        lazy(key, move |_| {
            let mut list = column![].spacing(2);
            for (cid, name, online, unread, chosen) in &rows {
                list = list.push(widgets::contact_row(*cid, name, *online, *unread, *chosen));
            }
            list
        })
        .into()
    };

    container(list)
        .width(Length::Fixed(260.0))
        .height(Length::Fill)
        .padding(Padding {
            top: 4.0,
            right: 4.0,
            bottom: 4.0,
            left: 4.0,
        })
        .style(palette::panel)
        .into()
}

fn main_pane<L: CoreLink>(state: &State<L>) -> Element<'_, Message> {
    let Some(cid) = state.store.selected() else {
        return container(text("Выберите чат слева").size(13).color(palette::DIM))
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center)
            .into();
    };

    let name = state
        .store
        .contact(cid)
        .map(|c| c.name.clone())
        .unwrap_or_else(|| cid.to_string());
    let online = state.store.contact(cid).map(|c| c.online).unwrap_or(false);

    let header = container(
        row![
            text(name)
                .size(13)
                .font(palette::bold())
                .color(palette::TEXT),
            space::Space::new().width(Length::Fill),
            text(if online {
                "● сессия"
            } else {
                "○ нет связи"
            })
            .size(11)
            .color(if online { palette::OK } else { palette::DIM }),
        ]
        .align_y(Vertical::Center),
    )
    .width(Length::Fill)
    .height(Length::Fixed(36.0))
    .padding(Padding {
        top: 0.0,
        right: 12.0,
        bottom: 0.0,
        left: 12.0,
    })
    .style(palette::panel);

    let feed = feed_view(state, cid);

    column![header, feed, composer(state)]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn feed_view<L: CoreLink>(state: &State<L>, cid: Cid) -> Element<'_, Message> {
    let items = state.store.feed(cid, local_day);
    let mut list = column![].spacing(2).padding(Padding {
        top: 8.0,
        right: 12.0,
        bottom: 8.0,
        left: 12.0,
    });
    for item in items {
        match item {
            FeedItem::DateSep { ts_ms } => list = list.push(widgets::date_sep(ts_ms)),
            FeedItem::Sys { text } => list = list.push(widgets::sys_line(text)),
            group @ FeedItem::Group { .. } => {
                for element in widgets::message_group(&group) {
                    list = list.push(element);
                }
            }
        }
    }

    scrollable(list)
        .id(state.feed_id.clone())
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn composer<L: CoreLink>(state: &State<L>) -> Element<'_, Message> {
    let editor = text_editor(&state.composer)
        .id(state.composer_id.clone())
        .height(Length::Fixed(96.0))
        .padding(Padding {
            top: 8.0,
            right: 0.0,
            bottom: 8.0,
            left: 0.0,
        })
        .on_action(|action| Message::Chat(ChatIn::Edit(action)))
        .key_binding(key_binding);

    container(
        row![
            editor,
            container(
                text("Enter — отправить · Shift+Enter — перенос")
                    .size(11)
                    .color(palette::DIM)
            )
            .align_y(Vertical::Bottom)
            .padding(Padding {
                top: 0.0,
                right: 4.0,
                bottom: 8.0,
                left: 0.0
            }),
        ]
        .align_y(Vertical::Center)
        .padding(Padding {
            top: 0.0,
            right: 12.0,
            bottom: 0.0,
            left: 12.0,
        }),
    )
    .width(Length::Fill)
    .style(palette::root)
    .into()
}

/// Enter — отправить, Shift+Enter — перенос; остальное — как в iced.
fn key_binding(event: text_editor::KeyPress) -> Option<text_editor::Binding<Message>> {
    match event.key.as_ref() {
        keyboard::Key::Named(Named::Enter) => {
            if event.modifiers.shift() {
                Some(text_editor::Binding::Insert('\n'))
            } else {
                Some(text_editor::Binding::Custom(Message::Chat(ChatIn::Send)))
            }
        }
        _ => text_editor::Binding::from_key_press(event),
    }
}

fn status_bar<L: CoreLink>(state: &State<L>) -> Element<'static, Message> {
    let total = state.store.contacts().count();
    let online = state.store.online_count();
    container(
        text(format!("ядро: заглушка · сессий: {online} из {total}"))
            .size(11)
            .color(palette::DIM),
    )
    .width(Length::Fill)
    .height(Length::Fixed(24.0))
    .padding(Padding {
        top: 0.0,
        right: 12.0,
        bottom: 0.0,
        left: 12.0,
    })
    .align_y(Vertical::Center)
    .style(palette::panel)
    .into()
}

// ---------- время и локальные подписи (TEXT_STYLE.md §3) ----------

const MONTHS: [&str; 12] = [
    "января",
    "февраля",
    "марта",
    "апреля",
    "мая",
    "июня",
    "июля",
    "августа",
    "сентября",
    "октября",
    "ноября",
    "декабря",
];

fn datetime(ts_ms: i64) -> chrono::DateTime<chrono::Local> {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(ts_ms)
        .single()
        .unwrap_or_else(|| {
            chrono::Local
                .timestamp_millis_opt(ts_ms)
                .earliest()
                .expect("ts из ядра — валидное время")
        })
}

fn time_hm(ts_ms: i64) -> String {
    datetime(ts_ms).format("%H:%M").to_string()
}

/// Индекс локального календарного дня; для группировки важно
/// только равенство, поэтому достаточно инъективной свёртки даты.
fn local_day(ts_ms: i64) -> i64 {
    let date = datetime(ts_ms).date_naive();
    i64::from(date.year()) * 1000 + i64::from(date.ordinal())
}

/// «Сегодня», «Вчера», дальше «12 марта».
fn date_label(ts_ms: i64) -> String {
    let date = datetime(ts_ms).date_naive();
    let today = chrono::Local::now().date_naive();
    if date == today {
        "Сегодня".into()
    } else if date == today.pred_opt().unwrap_or(today) {
        "Вчера".into()
    } else {
        format!("{} {}", date.day(), MONTHS[date.month0() as usize])
    }
}
