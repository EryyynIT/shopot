//! UI: Elm-архитектура — один State, update() как единственное место
//! мутаций, view() — чистая функция над Store (GUI_DEV.md §3).

pub mod notify;
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
use crate::model::store::Store;

/// Лимит тела SEND_MESSAGE (str16 ≤ 4096, CORE-API.md §4.3).
const MAX_BODY: usize = 4096;

/// Первый запрос истории при открытии чата; догрузка растёт от него
/// до потолка ядра (GUI_DEV.md §3: порциями 100–200).
const HISTORY_LOAD: u32 = 100;

/// Сообщения приложения: канал ядра, действия чата, знакомство, настройки,
/// окно.
#[derive(Debug, Clone)]
pub enum Message {
    Link(LinkIn),
    Chat(ChatIn),
    Meet(MeetIn),
    Settings(SettingsIn),
    Window(WindowIn),
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
    /// Ctrl+1..9 — чат по номеру в порядке сайдбара (GUI_UI.md §4).
    SelectIndex(usize),
    Send,
    Edit(text_editor::Action),
    /// Ctrl+F — панель поиска по загруженному чату (S6).
    SearchOpen,
    /// Ввод в поисковой строке панели.
    SearchInput(String),
    /// Лента прокручена; `true` — докатились до верха: догрузить более
    /// старые сообщения (S6; окно GET_HISTORY растёт от загруженного
    /// до потолка ядра).
    ScrollTop(bool),
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

/// Действия настроек (S3, GUI_UI.md §4): приватность уведомлений,
/// компактная лента, масштаб шрифта.
#[derive(Debug, Clone)]
pub enum SettingsIn {
    /// Открыть панель настроек.
    Open,
    /// Галка «скрывать текст в уведомлениях».
    ToggleNotificationText(bool),
    /// Галка «компактная лента».
    ToggleCompact(bool),
    /// Кнопка масштаба: +шаг или −шаг (границы удерживает Store).
    ChangeFontScale(f32),
}

/// События окна и глобальные команды: фокус для правила уведомлений,
/// выход по Ctrl+Q (GUI_UI.md §4).
#[derive(Debug, Clone)]
pub enum WindowIn {
    /// Окно получило/потеряло фокус.
    Focus(bool),
    /// Закрыть приложение.
    Exit,
}

/// Оверлей поверх главного окна (GUI_UI.md §3); звонок появится в S7.
pub enum Overlay {
    None,
    /// QR-данные тяжёлые — в боксе, чтобы оверлей не раздувать.
    Invite(Box<InviteView>),
    Card(Cid),
    Menu(MenuView),
    /// Настройки: собственного стейта нет, всё живёт в Store.settings.
    Settings,
    /// Поиск по загруженному чату (Ctrl+F, S6).
    Search(SearchView),
}

/// Панель поиска: запрос живёт здесь, совпадения считает Store.
pub struct SearchView {
    pub query: String,
    /// Фокус в строку при открытии.
    pub input_id: iced::widget::Id,
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
    /// В фокусе ли окно: открытый в фокусе чат не уведомляет
    /// (GUI_UI.md §4). Подписку окна обслуживает main; по умолчанию
    /// считаем окно в фокусе.
    pub window_focused: bool,
    /// GET_HISTORY в полёте — скролл к верху не плодит повторных
    /// запросов, пока окно не доехало.
    pub history_in_flight: bool,
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
            window_focused: true,
            history_in_flight: false,
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
        Message::Settings(settings) => on_settings(state, settings),
        Message::Window(window) => on_window(state, window),
    }
}

/// События окна: фокус — данные для правила уведомлений, выход —
/// завершение рантайма iced.
fn on_window<L: CoreLink>(state: &mut State<L>, window: WindowIn) -> Task<Message> {
    match window {
        WindowIn::Focus(focused) => {
            state.window_focused = focused;
            Task::none()
        }
        WindowIn::Exit => iced::exit(),
    }
}

/// Настройки пишутся в Store сразу; рендер читает их в том же кадре.
fn on_settings<L: CoreLink>(state: &mut State<L>, settings: SettingsIn) -> Task<Message> {
    match settings {
        SettingsIn::Open => {
            state.overlay = Overlay::Settings;
            Task::none()
        }
        SettingsIn::ToggleNotificationText(on) => {
            state.store.settings.hide_notification_text = on;
            Task::none()
        }
        SettingsIn::ToggleCompact(on) => {
            state.store.settings.compact = on;
            Task::none()
        }
        SettingsIn::ChangeFontScale(delta) => {
            state.store.settings.change_font_scale(delta);
            Task::none()
        }
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
                    .push_incoming_core(cid, &name, msg_id, ts_ms, text.clone());
                let mut task = if opened {
                    scroll_feed(state)
                } else {
                    Task::none()
                };
                // Уведомление ОС: открытый в фокусе чат молчит (GUI_UI.md §4).
                if notify::should_notify(state.store.selected(), state.window_focused, cid) {
                    let title = state
                        .store
                        .contact(cid)
                        .map(|c| c.name.clone())
                        .unwrap_or_else(|| name.clone());
                    let body = notify::notification_body(
                        &text,
                        state.store.settings.hide_notification_text,
                    );
                    task = Task::batch([task, notify::os_notify(title, body)]);
                }
                task
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
            // История дописывается в начало ленты; позицию прокрутки не
            // трогаем — пользователь читает то же место (S6).
            (Request::History { cid, .. }, Ok(OkReply::History { total, records })) => {
                state.history_in_flight = false;
                state.store.load_history(cid, total, &records);
                Task::none()
            }
            (Request::History { .. }, Err(_)) => {
                state.history_in_flight = false;
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
        ChatIn::SelectIndex(index) => {
            let cid = state.store.contacts().nth(index).map(|c| c.cid);
            match cid {
                Some(cid) => select_chat(state, cid),
                None => Task::none(),
            }
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
        ChatIn::SearchOpen => {
            // Панель модальна; без выбранного чата искать нечего.
            if matches!(state.overlay, Overlay::None) && state.store.selected().is_some() {
                let input_id = iced::widget::Id::unique();
                state.overlay = Overlay::Search(SearchView {
                    query: String::new(),
                    input_id: input_id.clone(),
                });
                return operation::focus::<Message>(input_id);
            }
            Task::none()
        }
        ChatIn::SearchInput(text) => {
            if let Overlay::Search(view) = &mut state.overlay {
                view.query = text;
            }
            Task::none()
        }
        ChatIn::ScrollTop(at_top) => {
            if at_top {
                load_older(state)
            } else {
                Task::none()
            }
        }
    }
}

/// Открыть чат: непрочитанные обнулены, композер заряжен черновиком,
/// при первом открытии запрошена история (S6).
fn select_chat<L: CoreLink>(state: &mut State<L>, cid: Cid) -> Task<Message> {
    state.store.select(Some(cid));
    let draft = state.store.draft(cid).to_owned();
    state.composer = text_editor::Content::with_text(&draft);
    let mut tasks = vec![
        scroll_feed(state),
        operation::focus::<Message>(state.composer_id.clone()),
    ];
    if !state.store.history_requested(cid) && !state.history_in_flight {
        state.store.mark_history_requested(cid);
        state.history_in_flight = true;
        tasks.push(request_task(
            state,
            Request::History {
                cid,
                limit: HISTORY_LOAD,
            },
        ));
    }
    Task::batch(tasks)
}

/// Скролл к верху ленты: расширить окно истории. Повторные события
/// скролла глушатся флагом в полёте и потолком окна ядра.
fn load_older<L: CoreLink>(state: &mut State<L>) -> Task<Message> {
    let Some(cid) = state.store.selected() else {
        return Task::none();
    };
    if state.history_in_flight || !state.store.has_more_history(cid) {
        return Task::none();
    }
    let limit =
        (state.store.history_loaded(cid) + HISTORY_LOAD).min(crate::core_link::HISTORY_MAX_LIMIT);
    state.history_in_flight = true;
    request_task(state, Request::History { cid, limit })
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
    let mut window = column![
        title_bar(state),
        row![sidebar(state), main_pane(state)].height(Length::Fill),
    ]
    .width(Length::Fill)
    .height(Length::Fill);
    if let Some(bar) = status_bar(state) {
        window = window.push(bar);
    }

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
            centered(widgets::invite_panel(view, state.store.settings.font_scale))
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
                centered(widgets::contact_card(
                    *cid,
                    &name,
                    online,
                    blocked,
                    state.store.settings.font_scale
                )),
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
                container(opaque(widgets::context_menu(
                    menu,
                    &name,
                    blocked,
                    state.store.settings.font_scale
                )))
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
        Overlay::Settings => stack![
            widgets::backdrop(close()),
            centered(widgets::settings_panel(
                &state.store.settings,
                state.store.settings.font_scale
            ))
        ]
        .into(),
        Overlay::Search(view) => {
            let hits = state
                .store
                .selected()
                .map_or_else(Vec::new, |cid| state.store.search_loaded(cid, &view.query));
            stack![
                widgets::backdrop(close()),
                centered(widgets::search_panel(
                    view,
                    hits,
                    state.store.settings.font_scale
                ))
            ]
            .into()
        }
    }
}

fn title_bar<L: CoreLink>(state: &State<L>) -> Element<'static, Message> {
    let scale = state.store.settings.font_scale;
    container(
        row![
            text("shopot")
                .size(14.0 * scale)
                .font(palette::bold())
                .color(palette::ACCENT),
            space::Space::new().width(Length::Fill),
            button(text("Настройки").size(11.0 * scale).color(palette::TEXT))
                .padding(Padding {
                    top: 2.0,
                    right: 10.0,
                    bottom: 2.0,
                    left: 10.0,
                })
                .style(palette::button_plain)
                .on_press(Message::Settings(SettingsIn::Open)),
        ]
        .spacing(10)
        .align_y(Vertical::Center),
    )
    .width(Length::Fill)
    .height(Length::Fixed(40.0))
    .align_y(Vertical::Center)
    .padding(Padding {
        top: 0.0,
        right: 16.0,
        bottom: 0.0,
        left: 16.0,
    })
    .style(palette::panel)
    .into()
}

fn sidebar<L: CoreLink>(state: &State<L>) -> Element<'_, Message> {
    let scale = state.store.settings.font_scale;
    let selected = state.store.selected();
    // Снимок строк + ключ ленивости: список перестраивается только при
    // изменении состава/статусов или масштаба (GUI_DEV.md §3 — lazy).
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
                    .size(12.0 * scale)
                    .color(palette::DIM),
                row![
                    button(text("Пригласить").size(12.0 * scale).color(palette::TEXT))
                        .padding(Padding {
                            top: 6.0,
                            right: 12.0,
                            bottom: 6.0,
                            left: 12.0,
                        })
                        .style(palette::button_dim)
                        .on_press(Message::Meet(MeetIn::OpenInvite)),
                    button(text("Добавить").size(12.0 * scale).color(palette::TEXT))
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
        // Масштаб в ключе: смена настройки обязана перерисовать строки.
        let key: Vec<(u32, i64, u32, bool, bool)> = rows
            .iter()
            .map(|(cid, _, online, unread, chosen)| {
                (scale.to_bits(), cid.0, *unread, *online, *chosen)
            })
            .collect();
        lazy(key, move |_| {
            let mut list = column![].spacing(2);
            for (cid, name, online, unread, chosen) in &rows {
                // Правая кнопка — меню знакомства; on_move запоминает точку.
                list = list.push(
                    mouse_area(widgets::contact_row(
                        *cid, name, *online, *unread, *chosen, scale,
                    ))
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
            button(text("Пригласить").size(12.0 * scale).color(palette::DIM))
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
            top: 8.0,
            right: 8.0,
            bottom: 8.0,
            left: 8.0,
        })
        .style(palette::panel)
        .into()
}

fn main_pane<L: CoreLink>(state: &State<L>) -> Element<'_, Message> {
    let scale = state.store.settings.font_scale;
    let Some(cid) = state.store.selected() else {
        return container(
            text("Выберите чат слева")
                .size(13.0 * scale)
                .color(palette::DIM),
        )
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
                        .size(14.0 * scale)
                        .font(palette::bold())
                        .color(palette::TEXT),
                    {
                        // Presence и сессия здесь одно и то же: ядро не
                        // шлёт сессию отдельно, она выводится из PRESENCE
                        // (store.rs). Пользователю — простые слова.
                        let (label, color) = if online {
                            ("в сети", palette::OK)
                        } else {
                            ("не в сети", palette::DIM)
                        };
                        text(label).size(10.0 * scale).color(color)
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
        ]
        .spacing(8)
        .align_y(Vertical::Center),
    )
    .width(Length::Fill)
    .height(Length::Fixed(52.0))
    .align_y(Vertical::Center)
    .padding(Padding {
        top: 0.0,
        right: 16.0,
        bottom: 0.0,
        left: 16.0,
    })
    .style(palette::panel);

    let feed = feed_view(state, cid);

    // Рамок у панелей больше нет: зазор между сайдбаром и лентой
    // не нужен, их разделяет контраст заливок (palette::panel/root).
    container(column![header, feed, composer(state)])
        .width(Length::Fill)
        .height(Length::Fill)
        .style(palette::root)
        .into()
}

fn feed_view<L: CoreLink>(state: &State<L>, cid: Cid) -> Element<'_, Message> {
    let scale = state.store.settings.font_scale;
    let compact = state.store.settings.compact;
    let items = state.store.feed(cid, local_day);
    let mut list = column![].spacing(2).padding(Padding {
        top: 8.0,
        right: 12.0,
        bottom: 8.0,
        left: 12.0,
    });
    for item in items {
        match item {
            FeedItem::DateSep { ts_ms } => list = list.push(widgets::date_sep(ts_ms, scale)),
            FeedItem::Sys { text } => list = list.push(widgets::sys_line(text, scale)),
            group @ FeedItem::Group { .. } => {
                let rows = if compact {
                    widgets::compact_group(&group, scale)
                } else {
                    widgets::message_group(&group, scale)
                };
                for element in rows {
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
        .on_scroll(|viewport| Message::Chat(ChatIn::ScrollTop(viewport.absolute_offset().y <= 0.0)))
        .into()
}

fn composer<L: CoreLink>(state: &State<L>) -> Element<'_, Message> {
    let scale = state.store.settings.font_scale;
    let empty = state.composer.text().trim().is_empty();
    // Авторост: базовая строка 48 px, до 5 строк — GUI_UI.md §1;
    // высота растёт вместе с масштабом шрифта.
    let lines = state.composer.text().lines().count().clamp(1, 5) as f32;
    let editor_height = (48.0 + (lines - 1.0) * 20.0) * scale;

    let editor = text_editor(&state.composer)
        .id(state.composer_id.clone())
        .height(Length::Fixed(editor_height))
        .size(14.0 * scale)
        .placeholder("Сообщение")
        .padding(Padding {
            top: 10.0,
            right: 12.0,
            bottom: 10.0,
            left: 12.0,
        })
        .on_action(|action| Message::Chat(ChatIn::Edit(action)))
        .key_binding(key_binding)
        .style(palette::editor);

    let send = button(text("↑").size(16.0 * scale).font(palette::bold()))
        .padding(Padding {
            top: 8.0,
            right: 13.0,
            bottom: 8.0,
            left: 13.0,
        })
        .style(palette::button_accent)
        .on_press_maybe((!empty).then_some(Message::Chat(ChatIn::Send)));

    container(
        iced::widget::Row::new()
            .push(container(editor).width(Length::Fill))
            .push(send)
            .spacing(10)
            .align_y(Vertical::Bottom),
    )
    .width(Length::Fill)
    .padding(Padding {
        top: 10.0,
        right: 16.0,
        bottom: 10.0,
        left: 16.0,
    })
    .style(palette::root)
    .into()
}

/// Ctrl+1..9 — индекс чата в порядке сайдбара. Код физической клавиши:
/// не зависит от раскладки (второй ряд и в RU, и в US — цифры).
pub fn chat_index(code: &iced::keyboard::key::Code) -> Option<usize> {
    use iced::keyboard::key::Code;
    match code {
        Code::Digit1 => Some(0),
        Code::Digit2 => Some(1),
        Code::Digit3 => Some(2),
        Code::Digit4 => Some(3),
        Code::Digit5 => Some(4),
        Code::Digit6 => Some(5),
        Code::Digit7 => Some(6),
        Code::Digit8 => Some(7),
        Code::Digit9 => Some(8),
        _ => None,
    }
}

/// Enter — отправить, Shift+Enter — перенос, Esc — закрыть оверлей,
/// Ctrl+1..9 — чат по номеру, Ctrl+F — поиск, Ctrl+Q — выход; остальное
/// — как в iced. Эти комбинации ловятся при фокусе композера: подписка
/// клавиатуры в headless-эмуляторе событий не получает.
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
        _ if event.modifiers.control() => ctrl_binding(event),
        _ => text_editor::Binding::from_key_press(event),
    }
}

/// Ctrl-комбинации — по физической клавише, раскладка не важна
/// (GUI_UI.md §4). Нераспознанное отдаётся iced.
fn ctrl_binding(event: text_editor::KeyPress) -> Option<text_editor::Binding<Message>> {
    use iced::keyboard::key::{Code, Physical};
    let Physical::Code(code) = event.physical_key else {
        return text_editor::Binding::from_key_press(event);
    };
    if let Some(index) = chat_index(&code) {
        return Some(text_editor::Binding::Custom(Message::Chat(
            ChatIn::SelectIndex(index),
        )));
    }
    match code {
        Code::KeyQ => Some(text_editor::Binding::Custom(Message::Window(
            WindowIn::Exit,
        ))),
        Code::KeyF => Some(text_editor::Binding::Custom(Message::Chat(
            ChatIn::SearchOpen,
        ))),
        _ => text_editor::Binding::from_key_press(event),
    }
}

/// Статус-строка существует только ради проблемы: нормальная работа не
/// занимает место на экране. Заглушка (демо без ядра) проблемой не
/// считается — там и показывать нечего.
fn status_bar<L: CoreLink>(state: &State<L>) -> Option<Element<'static, Message>> {
    let scale = state.store.settings.font_scale;
    if state.link.is_stub() || state.link_up {
        return None;
    }
    Some(
        container(
            text("Нет соединения. Сообщения не отправляются.")
                .size(11.0 * scale)
                .color(palette::ERR),
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
        .into(),
    )
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core_link::mock::{MockLink, Scene};
    use crate::model::store::Contact;

    fn state() -> State<MockLink> {
        State::new(Arc::new(MockLink::new(Scene::smoke())))
    }

    fn contact(cid: i64, name: &str) -> Contact {
        Contact {
            cid: Cid(cid),
            name: name.to_owned(),
            online: true,
            blocked: false,
        }
    }

    /// Ctrl+1..9 в headless-эмуляторе не вводятся — путь проверяем
    /// напрямую через update() (привязка композера и подписка ведут в
    /// тот же ChatIn::SelectIndex).
    #[test]
    fn select_index_opens_chat_in_sidebar_order() {
        let mut state = state();
        state.store.replace_contacts(vec![
            contact(1, "Аня"),
            contact(2, "Борис"),
            contact(3, "Вера"),
        ]);

        let _ = update(&mut state, Message::Chat(ChatIn::SelectIndex(2)));
        assert_eq!(state.store.selected(), Some(Cid(3)));
        let _ = update(&mut state, Message::Chat(ChatIn::SelectIndex(0)));
        assert_eq!(state.store.selected(), Some(Cid(1)));
    }

    #[test]
    fn select_index_beyond_contacts_is_noop() {
        let mut state = state();
        state.store.replace_contacts(vec![contact(1, "Аня")]);

        let _ = update(&mut state, Message::Chat(ChatIn::SelectIndex(5)));
        assert_eq!(state.store.selected(), None, "лишний номер молчится");
    }

    /// Поток истории S6: первое открытие чата запрашивает GET_HISTORY,
    /// ответ восстанавливает ленту, скролл к верху догружает старое,
    /// после исчерпания окна запросы не уходят.
    #[test]
    fn chat_open_restores_history_and_top_scroll_pages_older() {
        let mut state = state();
        state.store.replace_contacts(vec![contact(1, "Аня")]);
        let _ = update(&mut state, Message::Chat(ChatIn::Select(Cid(1))));
        assert!(state.store.history_requested(Cid(1)), "история запрошена");
        assert!(state.history_in_flight);

        let record = |n: u8, ts: i64, text: &str| crate::core_link::HistoryMsg {
            mine: false,
            delivered: false,
            msg_id: crate::model::msg::CoreMsgId([n; 16]),
            ts_ms: ts,
            text: text.to_owned(),
        };
        let history_reply = |records: Vec<crate::core_link::HistoryMsg>| {
            Message::Link(LinkIn::Replied {
                request: Request::History {
                    cid: Cid(1),
                    limit: 100,
                },
                reply: Ok(OkReply::History { total: 3, records }),
                tag: None,
            })
        };

        let _ = update(
            &mut state,
            history_reply(vec![record(1, 1000, "раз"), record(2, 2000, "два")]),
        );
        assert!(!state.history_in_flight);
        let msgs = &state.store.thread(Cid(1)).unwrap().msgs;
        assert_eq!(msgs.len(), 2, "лента восстановлена");
        assert!(state.store.has_more_history(Cid(1)));

        // Скролл к верху: окно растёт до потолка, старое встаёт впереди.
        let _ = update(&mut state, Message::Chat(ChatIn::ScrollTop(true)));
        assert!(state.history_in_flight, "догрузка запрошена");
        let _ = update(
            &mut state,
            history_reply(vec![record(0, 500, "самое старое")]),
        );
        assert_eq!(
            state.store.thread(Cid(1)).unwrap().msgs[0].body,
            "самое старое"
        );
        assert!(!state.store.has_more_history(Cid(1)), "total 3 исчерпан");

        // Верх без остатка — запрос не уходит.
        let _ = update(&mut state, Message::Chat(ChatIn::ScrollTop(true)));
        assert!(!state.history_in_flight, "нечего догружать");

        // Скролл не у верха глушится сразу.
        let _ = update(&mut state, Message::Chat(ChatIn::ScrollTop(false)));
        assert!(!state.history_in_flight);
    }

    /// Ctrl+F: панель открывается только при выбранном чате, ввод
    /// попадает в запрос, Esc-путь закрывает общий CloseOverlay.
    #[test]
    fn search_panel_opens_on_selected_chat_and_takes_input() {
        let mut state = state();
        state.store.replace_contacts(vec![contact(1, "Аня")]);

        let _ = update(&mut state, Message::Chat(ChatIn::SearchOpen));
        assert!(
            !matches!(state.overlay, Overlay::Search(_)),
            "без чата искать нечего"
        );

        let _ = update(&mut state, Message::Chat(ChatIn::Select(Cid(1))));
        let _ = update(&mut state, Message::Chat(ChatIn::SearchOpen));
        assert!(
            matches!(state.overlay, Overlay::Search(_)),
            "панель открыта"
        );

        let _ = update(
            &mut state,
            Message::Chat(ChatIn::SearchInput("привет".into())),
        );
        let Overlay::Search(view) = &state.overlay else {
            panic!("панель ещё открыта")
        };
        assert_eq!(view.query, "привет");
    }

    #[test]
    fn chat_index_maps_digit_row() {
        use iced::keyboard::key::Code;
        for (code, index) in [(Code::Digit1, 0), (Code::Digit5, 4), (Code::Digit9, 8)] {
            assert_eq!(chat_index(&code), Some(index));
        }
        assert_eq!(chat_index(&Code::Digit0), None, "Ctrl+0 не назначен");
        assert_eq!(chat_index(&Code::KeyA), None);
    }

    // ---------- render-тесты: view() в headless-дерево ----------
    //
    // Покрывают состояния, которые эмулятор ввести не умеет (Ctrl+F,
    // скролл ленты): состояние собирается через update() и Store
    // напрямую, затем проверяется точный текст виджетов. Без рантайма.

    /// view() состояния в дерево виджетов; find ищет точный текст.
    fn render<L: CoreLink>(state: &State<L>) -> iced_test::Simulator<'_, Message> {
        iced_test::simulator(view(state))
    }

    fn history_record(id: u8, ts_ms: i64, text: &str) -> crate::core_link::HistoryMsg {
        crate::core_link::HistoryMsg {
            mine: false,
            delivered: false,
            msg_id: crate::model::msg::CoreMsgId([id; 16]),
            ts_ms,
            text: text.to_owned(),
        }
    }

    /// Панель Ctrl+F: заголовок и подсказка на пустом запросе, честный
    /// «пусто» на промахе. Панель рендерится только открытой.
    #[test]
    fn search_panel_renders_hint_and_empty_state() {
        let mut state = state();
        state.store.replace_contacts(vec![contact(1, "Аня")]);
        state.store.select(Some(Cid(1)));
        let _ = update(&mut state, Message::Chat(ChatIn::SearchOpen));

        {
            let mut ui = render(&state);
            assert!(ui.find("Поиск по чату").is_ok(), "заголовок панели");
            assert!(
                ui.find("Поиск ищет по загруженным сообщениям этого чата.")
                    .is_ok(),
                "подсказка на пустом запросе"
            );
        }

        let _ = update(
            &mut state,
            Message::Chat(ChatIn::SearchInput("неттакого".into())),
        );
        let mut ui = render(&state);
        assert!(ui.find("Ничего не найдено.").is_ok(), "промах честно пуст");
    }

    /// Хвост панели: показаны первые 50 совпадений, остаток — строкой.
    #[test]
    fn search_results_capped_with_tail_line() {
        let mut state = state();
        state.store.replace_contacts(vec![contact(1, "Аня")]);
        let records: Vec<crate::core_link::HistoryMsg> = (1..=60u8)
            .map(|i| {
                history_record(
                    i,
                    1_000 + i64::from(i) * 1_000,
                    &format!("заметка {i} про хвост"),
                )
            })
            .collect();
        state.store.load_history(Cid(1), 60, &records);
        state.store.select(Some(Cid(1)));
        let _ = update(&mut state, Message::Chat(ChatIn::SearchOpen));
        let _ = update(
            &mut state,
            Message::Chat(ChatIn::SearchInput("хвост".into())),
        );

        let mut ui = render(&state);
        assert!(ui.find("и ещё 10").is_ok(), "60 минус 50 показанных");
        assert!(
            ui.find("заметка 7 про хвост").is_ok(),
            "строка совпадения видна"
        );
    }

    /// Статусы отправки рендерятся честно (GUI_STYLE.md §2): разрывы
    /// длиннее пяти минут дают каждому сообщению свою группу, у каждой
    /// свой статус в подвале.
    #[test]
    fn feed_renders_honest_send_states() {
        let mut state = state();
        state.store.replace_contacts(vec![contact(1, "Аня")]);
        let _ = state.store.push_outgoing(Cid(1), 1_000, "уходит сейчас".into());
        let queued = state.store.push_outgoing(Cid(1), 6_000_000, "ждёт очереди".into());
        let delivered = state.store.push_outgoing(Cid(1), 12_000_000, "уехало".into());
        let failed = state.store.push_outgoing(Cid(1), 18_000_000, "не проехало".into());
        state.store.mark_queued(Cid(1), queued);
        state
            .store
            .mark_failed(Cid(1), failed, "ядро недоступно".to_owned());
        let core_id = crate::model::msg::CoreMsgId([7; 16]);
        state.store.mark_sent_core(Cid(1), delivered, core_id);
        state.store.mark_delivered(Cid(1), core_id);
        state.store.select(Some(Cid(1)));

        let mut ui = render(&state);
        assert!(ui.find("отправляю").is_ok(), "Sending рендерится");
        assert!(ui.find("отправится позже").is_ok(), "Queued рендерится");
        assert!(ui.find("доставлено").is_ok(), "Delivered рендерится");
        assert!(
            ui.find("не ушло: ядро недоступно").is_ok(),
            "Failed рендерится с причиной"
        );
    }

    /// Разделитель дня и служебные строки ядра видны в ленте.
    #[test]
    fn feed_renders_day_separator_and_sys_lines() {
        let mut state = state();
        state.store.replace_contacts(vec![contact(1, "Аня")]);
        let now = now_ms();
        state.store.push_sys(Cid(1), now, "сессия установлена".to_owned());
        let _ = state.store.push_outgoing(Cid(1), now, "привет".into());
        state.store.select(Some(Cid(1)));

        let mut ui = render(&state);
        assert!(ui.find("Сегодня").is_ok(), "разделитель дня");
        assert!(
            ui.find("сессия установлена").is_ok(),
            "служебная строка ядра"
        );
        assert!(ui.find("привет").is_ok(), "пузырь сообщения");
    }
}
