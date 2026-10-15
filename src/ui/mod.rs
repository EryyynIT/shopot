//! UI: Elm-архитектура — один State, update() как единственное место
//! мутаций, view() — чистая функция над Store (GUI_DEV.md §3).

pub mod palette;
pub mod widgets;

use std::sync::Arc;

use chrono::Datelike;
use iced::alignment::{Horizontal, Vertical};
use iced::keyboard::{self, key::Named};
use iced::widget::{
    button, column, container, lazy, mouse_area, opaque, operation, qr_code, row, scrollable,
    space, stack, text, text_editor,
};
use iced::{Element, Length, Padding, Point, Task};

use crate::core_link::{now_ms, CoreLink, LinkEvent, OkReply, Reply, Request};
use crate::model::cid::Cid;
use crate::model::feed::FeedItem;
use crate::model::msg::MsgId;
use crate::model::store::Session;
use crate::model::store::Store;

/// Лимит тела SEND_MESSAGE (str16 ≤ 4096, CORE-API.md §4.3).
const MAX_BODY: usize = 4096;

/// Сообщения приложения: канал ядра, действия чата, знакомство.
#[derive(Debug, Clone)]
pub enum Message {
    Link(LinkIn),
    Chat(ChatIn),
    Meet(MeetIn),
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

/// Действия знакомства: инвайт-диалог, карточка контакта, блокировка
/// (спринт S2, GUI_UI.md §1).
#[derive(Debug, Clone)]
pub enum MeetIn {
    /// Открыть инвайт-диалог: форма выдачи + LAN-код из кэша сессии.
    OpenInvite,
    /// Ввод адреса для кода (host:port; пусто — только локальная сеть).
    AddrInput(String),
    /// Галка «через relay»: соединение через релей, peer IP не видит.
    RelayToggled(bool),
    /// Выдать код (CREATE_INVITE с текущими адресом и relay).
    IssueInvite,
    /// Скопировать код приглашения в буфер обмена.
    CopyCode,
    /// Ввод чужого кода.
    ForeignInput(String),
    /// Гасим чужой код (ACCEPT_INVITE).
    SubmitForeign,
    /// Карточка контакта (из шапки чата или меню по правой кнопке).
    OpenCard(Cid),
    /// Блок/разблок по текущему состоянию контакта.
    ToggleBlock(Cid),
    /// Правая кнопка на строке контакта (GUI_UI.md §4).
    OpenMenu(Cid),
    /// Движение мыши над строкой контакта — позиция для меню.
    Cursor(Point),
    /// Esc или клик мимо панели.
    CloseOverlay,
}

/// Оверлей поверх главного окна (GUI_UI.md §3); звонок появится в S7.
pub enum Overlay {
    None,
    /// QR-данные тяжёлые — в боксе, чтобы оверлей не раздувать.
    Invite(Box<InviteView>),
    Card(Cid),
    Menu(MenuView),
}

/// Инвайт-диалог: мой код и чужой код (GUI_UI.md §1).
pub struct InviteView {
    /// Код ядра (CREATE_INVITE); на экране — начало и хвост, полный — в QR.
    pub code: Option<String>,
    /// QR по коду; строится один раз при ответе ядра.
    pub qr: Option<qr_code::Data>,
    /// Ошибка выдачи или гашения кода — текст для пользователя.
    pub error: Option<String>,
    /// Введённый чужой код.
    pub foreign: String,
    /// ACCEPT_INVITE в полёте — кнопка «Добавить» неактивна.
    pub submitting: bool,
    /// Адрес для кода: пусто — только локальная сеть (LAN-анонс ядра).
    pub addr: String,
    /// Выдать код через relay; требует адреса (CORE-API.md §4.2).
    pub relay: bool,
}

/// Меню по правой кнопке на контакте (GUI_UI.md §4).
pub struct MenuView {
    pub cid: Cid,
    pub at: Point,
}

/// Весь стейт приложения; дженерик по каналу с ядром — ui не знает,
/// заглушка это или UDS-транспорт.
pub struct State<L: CoreLink> {
    pub link: Arc<L>,
    pub store: Store,
    /// Жив ли канал с ядром (Down/Resync и неудачный Contacts).
    pub link_up: bool,
    /// Содержимое композера; текст черновика хранит Store.
    pub composer: text_editor::Content,
    pub feed_id: iced::widget::Id,
    pub composer_id: iced::widget::Id,
    /// Оверлей поверх окна.
    pub overlay: Overlay,
    /// Позиция курсора над строкой контакта — точка меню.
    pub cursor: Point,
    /// Код приглашения, выданный ядром в этой сессии. Кэш: ядро не отдаёт
    /// список живых инвайтов, повторное открытие диалога не тратит слот.
    /// Тратится при CONTACT_JOINED — код одноразовый (CORE-API.md §4.2).
    pub invite_code: Option<String>,
}

impl<L: CoreLink> State<L> {
    pub fn new(link: Arc<L>) -> Self {
        Self {
            link,
            store: Store::new(),
            link_up: true,
            composer: text_editor::Content::new(),
            feed_id: iced::widget::Id::unique(),
            composer_id: iced::widget::Id::unique(),
            overlay: Overlay::None,
            cursor: Point::ORIGIN,
            invite_code: None,
        }
    }

    /// Оверлей закрыт — инвариант для тестов и сценариев.
    pub fn overlay_closed(&self) -> bool {
        matches!(self.overlay, Overlay::None)
    }
}

pub fn update<L: CoreLink>(state: &mut State<L>, message: Message) -> Task<Message> {
    match message {
        Message::Link(link) => on_link(state, link),
        Message::Chat(chat) => on_chat(state, chat),
        Message::Meet(meet) => on_meet(state, meet),
    }
}

fn on_link<L: CoreLink>(state: &mut State<L>, link: LinkIn) -> Task<Message> {
    match link {
        LinkIn::Event(event) => match event {
            LinkEvent::MessageReceived {
                cid,
                name,
                msg_id,
                ts_ms,
                text,
            } => {
                let opened = state.store.selected() == Some(cid);
                state
                    .store
                    .push_incoming_core(cid, &name, msg_id, ts_ms, text);
                if opened {
                    scroll_feed(state)
                } else {
                    Task::none()
                }
            }
            LinkEvent::MessageDelivered { cid, msg_id } => {
                state.store.mark_delivered(cid, msg_id);
                Task::none()
            }
            LinkEvent::Presence { cid, online, .. } => {
                state.store.set_presence(cid, online);
                Task::none()
            }
            LinkEvent::ContactJoined { cid, name } => {
                if state.store.ensure_contact(cid, &name) {
                    state.store.set_presence(cid, true);
                    state.store.push_sys(
                        cid,
                        now_ms(),
                        format!("{name} подключился по вашему коду"),
                    );
                }
                // Какой-то из кодов потрачен: кэш мог стать уже погашенным.
                state.invite_code = None;
                Task::none()
            }
            LinkEvent::ContactProfile { cid, name, .. } => {
                state.store.rename_contact(cid, &name);
                Task::none()
            }
            LinkEvent::BlockedByPeer { cid } => {
                state
                    .store
                    .push_sys(cid, now_ms(), "контакт заблокировал вас".into());
                state.store.set_presence(cid, false);
                Task::none()
            }
            LinkEvent::Down => {
                state.link_up = false;
                Task::none()
            }
            LinkEvent::Resync => {
                state.link_up = true;
                let link = Arc::clone(&state.link);
                Task::perform(
                    async move { CoreLink::execute(&*link, Request::Contacts).await },
                    |reply| {
                        Message::Link(LinkIn::Replied {
                            request: Request::Contacts,
                            reply,
                            tag: None,
                        })
                    },
                )
            }
        },
        LinkIn::Replied {
            request,
            reply,
            tag,
        } => match (request, reply) {
            (Request::Contacts, Ok(OkReply::Contacts(contacts))) => {
                state.link_up = true;
                state.store.replace_contacts(contacts);
                Task::none()
            }
            (Request::Contacts, Err(_)) => {
                state.link_up = false;
                Task::none()
            }
            (
                Request::SendMsg { cid, .. },
                Ok(OkReply::Sent {
                    msg_id,
                    queued: false,
                }),
            ) => {
                if let Some(tag) = tag {
                    state.store.mark_sent_core(cid, tag, msg_id);
                }
                Task::none()
            }
            (Request::SendMsg { cid, .. }, Ok(OkReply::Sent { queued: true, .. })) => {
                if let Some(tag) = tag {
                    state.store.mark_queued(cid, tag);
                }
                Task::none()
            }
            (Request::SendMsg { cid, .. }, Err(err)) => {
                if let Some(tag) = tag {
                    state.store.mark_failed(cid, tag, err.user_text());
                }
                Task::none()
            }
            (Request::Invite { relay, addr }, Ok(OkReply::Code(code))) => {
                // Кэшируем только LAN-код; коды с адресом — разовое решение.
                if !relay && addr.is_empty() {
                    state.invite_code = Some(code.clone());
                }
                if let Overlay::Invite(view) = &mut state.overlay {
                    view.submitting = false;
                    view.code = Some(code.clone());
                    view.qr = qr_of(&code);
                }
                Task::none()
            }
            (Request::Invite { .. }, Err(err)) => {
                if let Overlay::Invite(view) = &mut state.overlay {
                    view.submitting = false;
                    view.error = Some(err.user_text());
                }
                Task::none()
            }
            (Request::Add { .. }, Ok(OkReply::Added { cid })) => {
                state.overlay = Overlay::None;
                Task::batch([
                    select_chat(state, cid),
                    // Сайдбар перечитает список: контакт появился у ядра.
                    request_task(state, Request::Contacts),
                ])
            }
            (Request::Add { .. }, Err(err)) => {
                if let Overlay::Invite(view) = &mut state.overlay {
                    view.submitting = false;
                    view.error = Some(err.user_text());
                }
                Task::none()
            }
            (Request::Block { cid }, Ok(OkReply::Done)) => {
                state.store.set_blocked(cid, true);
                Task::none()
            }
            (Request::Unblock { cid }, Ok(OkReply::Done)) => {
                state.store.set_blocked(cid, false);
                Task::none()
            }
            // Остальные ответы ядерного канала появляются со своими спринтами.
            _ => Task::none(),
        },
    }
}

fn on_chat<L: CoreLink>(state: &mut State<L>, chat: ChatIn) -> Task<Message> {
    match chat {
        ChatIn::Select(cid) => select_chat(state, cid),
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

/// Открыть чат: непрочитанные обнулены, композер заряжен черновиком.
fn select_chat<L: CoreLink>(state: &mut State<L>, cid: Cid) -> Task<Message> {
    state.store.select(Some(cid));
    let draft = state.store.draft(cid).to_owned();
    state.composer = text_editor::Content::with_text(&draft);
    Task::batch([
        scroll_feed(state),
        operation::focus::<Message>(state.composer_id.clone()),
    ])
}

/// Действия знакомства; все они идут через [`request_task`], ответы
/// маршрутизирует `on_link`.
fn on_meet<L: CoreLink>(state: &mut State<L>, meet: MeetIn) -> Task<Message> {
    match meet {
        MeetIn::OpenInvite => {
            state.overlay = Overlay::Invite(Box::new(InviteView {
                code: None,
                qr: None,
                error: None,
                foreign: String::new(),
                submitting: false,
                addr: String::new(),
                relay: false,
            }));
            // Кэш LAN-кода: не тратим слот реестра приглашений.
            if let Some(code) = state.invite_code.clone() {
                if let Overlay::Invite(view) = &mut state.overlay {
                    view.code = Some(code.clone());
                    view.qr = qr_of(&code);
                }
            }
            Task::none()
        }
        MeetIn::AddrInput(text) => {
            if let Overlay::Invite(view) = &mut state.overlay {
                view.addr = text;
                // relay без адреса не имеет смысла — снимаем галку.
                if view.addr.trim().is_empty() {
                    view.relay = false;
                }
            }
            Task::none()
        }
        MeetIn::RelayToggled(on) => {
            if let Overlay::Invite(view) = &mut state.overlay {
                view.relay = on && !view.addr.trim().is_empty();
            }
            Task::none()
        }
        MeetIn::IssueInvite => {
            let request = match &state.overlay {
                Overlay::Invite(view) if !view.submitting => Request::Invite {
                    relay: view.relay,
                    addr: view.addr.trim().to_owned(),
                },
                _ => return Task::none(),
            };
            if let Overlay::Invite(view) = &mut state.overlay {
                view.submitting = true;
                view.error = None;
            }
            request_task(state, request)
        }
        MeetIn::CopyCode => {
            let code = match &state.overlay {
                Overlay::Invite(view) => view.code.clone(),
                _ => None,
            };
            match code {
                Some(code) => iced::clipboard::write(code),
                None => Task::none(),
            }
        }
        MeetIn::ForeignInput(text) => {
            if let Overlay::Invite(view) = &mut state.overlay {
                view.foreign = text;
            }
            Task::none()
        }
        MeetIn::SubmitForeign => {
            let Overlay::Invite(view) = &mut state.overlay else {
                return Task::none();
            };
            let code = view.foreign.trim().to_owned();
            if code.is_empty() || view.submitting {
                return Task::none();
            }
            view.submitting = true;
            view.error = None;
            request_task(state, Request::Add { code })
        }
        MeetIn::OpenCard(cid) => {
            state.overlay = Overlay::Card(cid);
            Task::none()
        }
        MeetIn::ToggleBlock(cid) => {
            let blocked = state.store.contact(cid).is_some_and(|c| c.blocked);
            // Меню закрываем сразу; карточка остаётся открытой — статус
            // «заблокирован/в сети» обновится в ней после ответа ядра
            // (GUI_STYLE.md §2, честность статусов).
            if matches!(state.overlay, Overlay::Menu(_)) {
                state.overlay = Overlay::None;
            }
            if blocked {
                request_task(state, Request::Unblock { cid })
            } else {
                request_task(state, Request::Block { cid })
            }
        }
        MeetIn::OpenMenu(cid) => {
            state.overlay = Overlay::Menu(MenuView {
                cid,
                at: state.cursor,
            });
            Task::none()
        }
        MeetIn::Cursor(point) => {
            state.cursor = point;
            Task::none()
        }
        MeetIn::CloseOverlay => {
            state.overlay = Overlay::None;
            Task::none()
        }
    }
}

fn send_current<L: CoreLink>(state: &mut State<L>) -> Task<Message> {
    // Оверлей модален: Enter в композере за ним не отправляет.
    if !matches!(state.overlay, Overlay::None) {
        return Task::none();
    }
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

    // Превышение лимита ядра ловим до поездки: тело SEND_MESSAGE — str16.
    if text.len() > MAX_BODY {
        state.store.mark_failed(
            cid,
            msg_id,
            "Сообщение слишком длинное — лимит 4096 байт.".into(),
        );
        return Task::none();
    }

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

/// Разовый запрос ядру без привязки к пузырю (contacts/invite/add/block).
fn request_task<L: CoreLink>(state: &State<L>, request: Request) -> Task<Message> {
    let reply_request = request.clone();
    let link = Arc::clone(&state.link);
    Task::perform(link.execute(request), move |reply| {
        Message::Link(LinkIn::Replied {
            request: reply_request.clone(),
            reply,
            tag: None,
        })
    })
}

/// QR по коду; сбой построения не роняет диалог — остаётся копирование.
fn qr_of(code: &str) -> Option<qr_code::Data> {
    qr_code::Data::new(code.as_bytes()).ok()
}

/// Вид: окно целиком, оверлей поверх (GUI_UI.md §1).
pub fn view<L: CoreLink>(state: &State<L>) -> Element<'_, Message> {
    let window = column![
        title_bar(state),
        row![sidebar(state), main_pane(state)].height(Length::Fill),
        status_bar(state),
    ]
    .width(Length::Fill)
    .height(Length::Fill);

    let layered: Element<'_, Message> = match &state.overlay {
        Overlay::None => window.into(),
        overlay => stack![window, overlay_layer(state, overlay)].into(),
    };

    container(layered)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(palette::root)
        .into()
}

/// Оверлей: подложка-кнопка закрывает по клику мимо панели; сама панель
/// обёрнута в opaque — клики по её пустым местам не проваливаются под неё.
fn overlay_layer<'a, L: CoreLink>(
    state: &'a State<L>,
    overlay: &'a Overlay,
) -> Element<'a, Message> {
    let close = || Message::Meet(MeetIn::CloseOverlay);
    let centered = |panel: iced::Element<'a, Message>| {
        container(opaque(panel))
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center)
    };
    match overlay {
        Overlay::None => space::Space::new().into(),
        Overlay::Invite(view) => stack![
            widgets::backdrop(close()),
            centered(widgets::invite_panel(view))
        ]
        .into(),
        Overlay::Card(cid) => {
            let contact = state.store.contact(*cid);
            let name = contact
                .map(|c| c.name.clone())
                .unwrap_or_else(|| cid.to_string());
            let online = contact.map(|c| c.online).unwrap_or(false);
            let blocked = contact.map(|c| c.blocked).unwrap_or(false);
            stack![
                widgets::backdrop(close()),
                centered(widgets::contact_card(*cid, &name, online, blocked)),
            ]
            .into()
        }
        Overlay::Menu(menu) => {
            let contact = state.store.contact(menu.cid);
            let name = contact
                .map(|c| c.name.clone())
                .unwrap_or_else(|| menu.cid.to_string());
            let blocked = contact.map(|c| c.blocked).unwrap_or(false);
            // Меню у курсора; у нижней кромки окна — над курсором.
            let top = if menu.at.y > 380.0 {
                menu.at.y - 108.0
            } else {
                menu.at.y + 6.0
            };
            stack![
                widgets::backdrop(close()),
                container(opaque(widgets::context_menu(menu, &name, blocked)))
                    .padding(Padding {
                        top,
                        left: menu.at.x,
                        right: 0.0,
                        bottom: 0.0,
                    })
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .align_x(Horizontal::Left)
                    .align_y(Vertical::Top),
            ]
            .into()
        }
    }
}

fn title_bar<L: CoreLink>(state: &State<L>) -> Element<'static, Message> {
    let (label, color) = if state.link.is_stub() {
        ("режим: заглушка", palette::DIM)
    } else if state.link_up {
        ("ядро: API 1.0.0", palette::DIM)
    } else {
        ("ядро: нет связи", palette::ERR)
    };
    container(
        row![
            text("shopot")
                .size(14)
                .font(palette::bold())
                .color(palette::ACCENT),
            space::Space::new().width(Length::Fill),
            text(label).size(11).color(color),
        ]
        .align_y(Vertical::Center),
    )
    .width(Length::Fill)
    .height(Length::Fixed(32.0))
    .align_y(Vertical::Center)
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
    let has_contacts = !rows.is_empty();

    let list: Element<'static, Message> = if rows.is_empty() {
        container(
            column![
                text("Пока контактов нет. Создайте код приглашения или вставьте чужой.")
                    .size(12)
                    .color(palette::DIM),
                row![
                    button(text("Пригласить").size(12).color(palette::TEXT))
                        .padding(Padding {
                            top: 6.0,
                            right: 12.0,
                            bottom: 6.0,
                            left: 12.0,
                        })
                        .style(palette::button_dim)
                        .on_press(Message::Meet(MeetIn::OpenInvite)),
                    button(text("Добавить").size(12).color(palette::TEXT))
                        .padding(Padding {
                            top: 6.0,
                            right: 12.0,
                            bottom: 6.0,
                            left: 12.0,
                        })
                        .style(palette::button_dim)
                        .on_press(Message::Meet(MeetIn::OpenInvite)),
                ]
                .spacing(8)
            ]
            .spacing(10),
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
                // Правая кнопка — меню знакомства; on_move запоминает точку.
                list = list.push(
                    mouse_area(widgets::contact_row(*cid, name, *online, *unread, *chosen))
                        .on_right_press(Message::Meet(MeetIn::OpenMenu(*cid)))
                        .on_move(|point| Message::Meet(MeetIn::Cursor(point))),
                );
            }
            list
        })
        .into()
    };

    // Постоянный вход в инвайт-диалог, когда контакты уже есть.
    let mut pane = column![list].spacing(4);
    if has_contacts {
        pane = pane.push(
            button(text("Пригласить").size(12).color(palette::DIM))
                .width(Length::Fill)
                .padding(Padding {
                    top: 6.0,
                    right: 12.0,
                    bottom: 6.0,
                    left: 12.0,
                })
                .style(palette::button_dim)
                .on_press(Message::Meet(MeetIn::OpenInvite)),
        );
    }

    container(pane)
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
            // Клик по имени открывает карточку контакта (GUI_UI.md §1).
            button(
                column![
                    text(name.clone())
                        .size(14)
                        .font(palette::bold())
                        .color(palette::TEXT),
                    {
                        let (label, color) = match state.store.session(cid) {
                            Session::Up => ("защищённая сессия", palette::OK),
                            Session::Down => ("сессия не установлена", palette::WARN),
                            Session::Unknown => ("состояние сессии неизвестно", palette::MUTED),
                        };
                        text(label).size(10).color(color)
                    },
                ]
                .spacing(1)
                .align_x(Horizontal::Left),
            )
            .padding(Padding {
                top: 2.0,
                right: 6.0,
                bottom: 2.0,
                left: 6.0,
            })
            .style(palette::button_plain)
            .on_press(Message::Meet(MeetIn::OpenCard(cid))),
            space::Space::new().width(Length::Fill),
            text(if online { "●" } else { "○" })
                .size(12)
                .color(if online { palette::OK } else { palette::DIM }),
        ]
        .spacing(8)
        .align_y(Vertical::Center),
    )
    .width(Length::Fill)
    .height(Length::Fixed(48.0))
    .align_y(Vertical::Center)
    .padding(Padding {
        top: 0.0,
        right: 12.0,
        bottom: 0.0,
        left: 12.0,
    })
    .style(palette::panel);

    let feed = feed_view(state, cid);

    // Зазор слева: рамка шапки не должна прилегать к рамке сайдбара —
    // иначе читается как двойная линия (правка осмотра окна).
    container(column![header, feed, composer(state)])
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding {
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
            left: 4.0,
        })
        .style(palette::root)
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

    // Контент прижат к низу: короткая переписка не повисает в пустоте
    // над композером; при переполнении контейнер растёт и скроллится.
    let anchored = container(list)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_y(Vertical::Bottom);

    scrollable(anchored)
        .id(state.feed_id.clone())
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn composer<L: CoreLink>(state: &State<L>) -> Element<'_, Message> {
    let empty = state.composer.text().trim().is_empty();
    // Авторост: базовая строка 48 px, до 5 строк (128 px) — GUI_UI.md §1.
    let lines = state.composer.text().lines().count().clamp(1, 5) as f32;
    let editor_height = 48.0 + (lines - 1.0) * 20.0;

    let editor = text_editor(&state.composer)
        .id(state.composer_id.clone())
        .height(Length::Fixed(editor_height))
        .size(14)
        .placeholder("Сообщение… · Enter — отправить, Shift+Enter — перенос строки")
        .padding(Padding {
            top: 10.0,
            right: 12.0,
            bottom: 10.0,
            left: 12.0,
        })
        .on_action(|action| Message::Chat(ChatIn::Edit(action)))
        .key_binding(key_binding)
        .style(palette::editor);

    let send = button(
        text("Отправить")
            .size(12)
            .font(palette::bold())
            .color(palette::BG),
    )
    .padding(Padding {
        top: 10.0,
        right: 14.0,
        bottom: 10.0,
        left: 14.0,
    })
    .style(palette::button_accent)
    .on_press_maybe((!empty).then_some(Message::Chat(ChatIn::Send)));

    container(
        iced::widget::Row::new()
            .push(container(editor).width(Length::Fill))
            .push(send)
            .spacing(8)
            .align_y(Vertical::Bottom),
    )
    .width(Length::Fill)
    .padding(Padding {
        top: 8.0,
        right: 12.0,
        bottom: 8.0,
        left: 12.0,
    })
    .style(palette::root)
    .into()
}

/// Enter — отправить, Shift+Enter — перенос, Esc — закрыть оверлей;
/// остальное — как в iced. Esc здесь ловится при фокусе композера:
/// подписка клавиатуры в headless-эмуляторе событий не получает.
fn key_binding(event: text_editor::KeyPress) -> Option<text_editor::Binding<Message>> {
    match event.key.as_ref() {
        keyboard::Key::Named(Named::Enter) => {
            if event.modifiers.shift() {
                Some(text_editor::Binding::Insert('\n'))
            } else {
                Some(text_editor::Binding::Custom(Message::Chat(ChatIn::Send)))
            }
        }
        keyboard::Key::Named(Named::Escape) => Some(text_editor::Binding::Custom(Message::Meet(
            MeetIn::CloseOverlay,
        ))),
        _ => text_editor::Binding::from_key_press(event),
    }
}

fn status_bar<L: CoreLink>(state: &State<L>) -> Element<'static, Message> {
    let total = state.store.contacts().count();
    let online = state.store.online_count();
    let (line, color) = if !state.link.is_stub() && !state.link_up {
        (
            format!("ядро: нет связи · сессий: {online} из {total}"),
            palette::ERR,
        )
    } else {
        let mode = if state.link.is_stub() {
            "заглушка"
        } else {
            "UDS"
        };
        (
            format!("ядро: {mode} · сессий: {online} из {total}"),
            palette::DIM,
        )
    };
    container(text(line).size(11).color(color))
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

/// Код для показа: начало и хвост (GUI_UI.md §1 рисует «shopot1.Bq7…xRf»).
/// Полный код едет через QR и «Скопировать код», а не через текст на экране.
pub fn elided_code(code: &str) -> String {
    let chars: Vec<char> = code.chars().collect();
    if chars.len() <= 36 {
        return code.to_owned();
    }
    let head: String = chars[..16].iter().collect();
    let tail: String = chars[chars.len() - 16..].iter().collect();
    format!("{head}…{tail}")
}

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
