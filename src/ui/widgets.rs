//! Виджет-компоненты: не знают про протокол, читают только Store
//! и переданные данные (GUI_UI.md §2).

use iced::alignment::Horizontal;
use iced::widget::{button, column, container, qr_code, row, space, text, text_input};
use iced::{Element, Length, Padding};

use crate::model::cid::Cid;
use crate::model::feed::FeedItem;
use crate::model::msg::SendState;
use crate::model::settings::Settings;
use crate::ui::palette;
use crate::ui::{time_hm, ChatIn, InviteView, MeetIn, MenuView, Message, SettingsIn};

/// Строка сайдбара: акцентная полоса у самого края карточки, точка
/// presence, имя, бейдж непрочитанных. Высота карточки фиксированная —
/// полоса с Fill тянется ровно на неё, а не распирает весь список
/// (правка осмотра окна: строки растягивались на всю высоту сайдбара).
pub fn contact_row(
    cid: Cid,
    name: &str,
    online: bool,
    unread: u32,
    selected: bool,
    scale: f32,
) -> Element<'static, Message> {
    let bar = container(text(""))
        .width(Length::Fixed(3.0))
        .height(Length::Fill)
        .style(palette::chat_bar(selected));

    let dot = text(if online { "●" } else { "○" })
        .size(12.0 * scale)
        .color(if online { palette::OK } else { palette::DIM });

    let has_unread = unread > 0;
    // to_owned: фрагмент текста должен владеть строкой (Element 'static
    // для lazy), &str протянул бы заимствование из Store. База 16 —
    // прежний дефолт iced, принятый в S1 вид строки не меняем.
    let name = text(name.to_owned())
        .size(16.0 * scale)
        .font(if has_unread || selected {
            palette::bold()
        } else {
            iced::Font::default()
        })
        .color(if has_unread || selected {
            palette::TEXT
        } else {
            palette::DIM
        })
        .width(Length::Fill);

    let mut cells = row![bar, dot, name]
        .spacing(8)
        .height(Length::Fill)
        .align_y(iced::alignment::Vertical::Center);
    if has_unread {
        cells = cells.push(
            container(text(unread.to_string()).size(11.0 * scale))
                .padding(Padding {
                    top: 1.0,
                    right: 6.0,
                    bottom: 1.0,
                    left: 6.0,
                })
                .style(palette::badge),
        );
    }

    button(cells.padding(Padding {
        top: 0.0,
        right: 10.0,
        bottom: 0.0,
        left: 0.0,
    }))
    .on_press(Message::Chat(ChatIn::Select(cid)))
    .style(palette::chat_row(selected))
    .width(Length::Fill)
    .height(Length::Fixed(56.0))
    .into()
}

/// Разделитель дня по центру: «Сегодня», «Вчера», «12 марта».
pub fn date_sep(ts_ms: i64, scale: f32) -> Element<'static, Message> {
    container(
        text(crate::ui::date_label(ts_ms))
            .size(11.0 * scale)
            .color(palette::DIM),
    )
    .width(Length::Fill)
    .align_x(Horizontal::Center)
    .padding(Padding {
        top: 4.0,
        right: 0.0,
        bottom: 4.0,
        left: 0.0,
    })
    .into()
}

/// Служебная строка ядра: цветом WARN — событие важное для безопасности
/// и не должно сливаться с разделителями дат.
pub fn sys_line(body: String, scale: f32) -> Element<'static, Message> {
    container(text(body).size(11.0 * scale).color(palette::WARN))
        .width(Length::Fill)
        .align_x(Horizontal::Center)
        .padding(Padding {
            top: 2.0,
            right: 0.0,
            bottom: 2.0,
            left: 0.0,
        })
        .into()
}

/// Статус отправки моей группы — подпись в подвале (честность статусов,
/// GUI_STYLE.md §2). У чужой группы статуса нет.
fn send_status(send: &SendState) -> Option<(String, iced::Color)> {
    match send {
        SendState::Sending => Some(("отправляю".to_owned(), palette::DIM)),
        SendState::Sent => Some(("отправлено".to_owned(), palette::DIM)),
        SendState::Queued => Some(("отправится позже".to_owned(), palette::DIM)),
        SendState::Delivered => Some(("доставлено".to_owned(), palette::OK)),
        SendState::Failed(reason) => Some((format!("не ушло: {reason}"), palette::ERR)),
    }
}

/// Подвал группы: время и статус отправки (только у меня).
fn group_footer(
    ts_ms: i64,
    status: &Option<(String, iced::Color)>,
    scale: f32,
) -> iced::Element<'static, Message> {
    let mut foot = row![text(time_hm(ts_ms))
        .size(10.0 * scale)
        .color(palette::MUTED)]
    .spacing(4.0);
    if let Some((label, color)) = status {
        foot = foot.push(text(label.clone()).size(10.0 * scale).color(*color));
    }
    foot.into()
}

/// Обёртка строки группы: ширина ленты, отступ, выравнивание.
fn wrap_row(
    inner: iced::Element<'static, Message>,
    align_x: Horizontal,
) -> Element<'static, Message> {
    container(inner)
        .width(Length::Fill)
        .align_x(align_x)
        .padding(Padding {
            top: 1.0,
            right: 2.0,
            bottom: 1.0,
            left: 2.0,
        })
        .into()
}

/// Группа сообщений одного автора: пузыри моей стороны справа, чужие
/// слева; тексты в пузыре — по левому краю; время и статус — строкой
/// под пузырём, у его правой кромки. Fill-распорок в цепочке группы
/// нет ни одной: они протягивают ширину ближайшего ограничителя и
/// распахивают пузырь (правка по замерам пробы раскладки).
pub fn message_group(group: &FeedItem, scale: f32) -> Vec<Element<'static, Message>> {
    let FeedItem::Group {
        mine,
        name: _,
        ts_ms,
        bodies,
        send,
        ..
    } = group
    else {
        return Vec::new();
    };

    let align_x = if *mine {
        Horizontal::Right
    } else {
        Horizontal::Left
    };

    let status = if *mine { send_status(send) } else { None };

    let bubble_style: fn(&iced::Theme) -> iced::widget::container::Style = if *mine {
        palette::bubble_mine
    } else {
        palette::bubble_theirs
    };
    let bubble = |col: iced::widget::Column<'static, Message>| {
        container(col)
            .max_width(560)
            .padding(Padding {
                top: 5.0,
                right: 10.0,
                bottom: 5.0,
                left: 10.0,
            })
            .style(bubble_style)
    };

    let mut elements: Vec<Element<'static, Message>> = Vec::with_capacity(bodies.len());
    let mut pending: Option<iced::widget::Column<'static, Message>> = None;
    let count = bodies.len();

    for (index, (_, body)) in bodies.iter().enumerate() {
        let last = index + 1 == count;
        let body_text = text(body.to_owned())
            .size(14.0 * scale)
            .color(palette::TEXT);

        // Длинные тексты — без пузыря, на всю ширину (GUI_STYLE.md §2).
        if body.len() > 300 {
            if let Some(col) = pending.take() {
                elements.push(wrap_row(bubble(col).into(), align_x));
            }
            let block = iced::widget::Column::new().push(
                container(body_text)
                    .max_width(560)
                    .padding(Padding {
                        top: 3.0,
                        right: 4.0,
                        bottom: 3.0,
                        left: 4.0,
                    })
                    .style(palette::no_bubble),
            );
            if last {
                elements.push(wrap_row(
                    group_with_footer(block, group_footer(*ts_ms, &status, scale)),
                    align_x,
                ));
            } else {
                elements.push(wrap_row(block.into(), align_x));
            }
        } else {
            let col = pending
                .take()
                .unwrap_or_else(|| iced::widget::Column::new().spacing(2));
            pending = Some(col.push(body_text));
        }
    }
    if let Some(col) = pending.take() {
        elements.push(wrap_row(
            group_with_footer(bubble(col), group_footer(*ts_ms, &status, scale)),
            align_x,
        ));
    }

    elements
}

/// Компактная группа: без пузырей, всё слева, мои строки — с отступом
/// (GUI_STYLE.md §2). Имя автора — над чужой группой: выравнивание
/// больше не отличает авторов.
pub fn compact_group(group: &FeedItem, scale: f32) -> Vec<Element<'static, Message>> {
    let FeedItem::Group {
        mine,
        name,
        ts_ms,
        bodies,
        send,
        ..
    } = group
    else {
        return Vec::new();
    };

    let status = if *mine { send_status(send) } else { None };
    let mut col = iced::widget::Column::<Message>::new().spacing(2);
    if !*mine {
        col = col.push(
            text(name.clone())
                .size(12.0 * scale)
                .font(palette::bold())
                .color(palette::DIM),
        );
    }
    let indent = if *mine { 16.0 } else { 0.0 };
    for (_, body) in bodies {
        col = col.push(
            container(
                text(body.to_owned())
                    .size(14.0 * scale)
                    .color(palette::TEXT),
            )
            .padding(Padding {
                left: indent,
                ..Padding::default()
            }),
        );
    }
    col = col.push(group_footer(*ts_ms, &status, scale));

    vec![wrap_row(col.into(), Horizontal::Left)]
}

/// Пузырь (или длинный блок) + подвал под ним: подвал прижат к правой
/// кромке самого широкого элемента группы — колонка Shrink, оба ребёнка
/// Shrink, align_x(Right) сводит их правые кромки.
fn group_with_footer(
    main: impl Into<iced::Element<'static, Message>>,
    footer: iced::Element<'static, Message>,
) -> iced::Element<'static, Message> {
    iced::widget::Column::new()
        .push(main.into())
        .push(container(footer).padding(Padding {
            top: 1.0,
            right: 0.0,
            bottom: 0.0,
            left: 0.0,
        }))
        .align_x(Horizontal::Right)
        .into()
}

// ---------- оверлеи (GUI_UI.md §1, спринт S2) ----------

/// Подложка оверлея: клик мимо панели закрывает её.
pub fn backdrop(on_close: Message) -> Element<'static, Message> {
    container(
        button(
            iced::widget::space::Space::new()
                .width(Length::Fill)
                .height(Length::Fill),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .style(palette::button_invisible)
        .on_press(on_close),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .style(palette::backdrop)
    .into()
}

/// Инвайт-диалог: мой код (эллипсис + QR + копирование) и чужой код.
/// Возвращаемый Element заимствует view — оверлей не lazy.
pub fn invite_panel(view: &InviteView, scale: f32) -> Element<'_, Message> {
    let mut body = column![text("Приглашение")
        .size(14.0 * scale)
        .font(palette::bold())
        .color(palette::TEXT),]
    .spacing(10);

    // Форма выдачи: адрес опционален; relay только с адресом (CORE-API §4.2).
    body = body.push(text("Адрес").size(11.0 * scale).color(palette::DIM));
    body = body.push(
        text_input("host:7777 — необязательно", &view.addr)
            .on_input(|value| Message::Meet(MeetIn::AddrInput(value)))
            .size(12.0 * scale)
            .padding(8)
            .style(palette::input),
    );
    body = body.push(
        iced::widget::checkbox(view.relay)
            .label("через relay — собеседник не увидит ваш IP")
            .on_toggle(|on| Message::Meet(MeetIn::RelayToggled(on)))
            .text_size(11.0 * scale)
            .style(palette::checkbox_style),
    );
    body = body.push(
        text("Без адреса код работает только в одной локальной сети.")
            .size(10.0 * scale)
            .color(palette::DIM),
    );
    body = body.push(
        button(
            text("Выдать код")
                .size(12.0 * scale)
                .font(palette::bold())
                .color(palette::BG),
        )
        .padding(Padding {
            top: 8.0,
            right: 14.0,
            bottom: 8.0,
            left: 14.0,
        })
        .style(palette::button_accent)
        .on_press_maybe((!view.submitting).then_some(Message::Meet(MeetIn::IssueInvite))),
    );

    body = body.push(text("Мой код").size(11.0 * scale).color(palette::DIM));
    body = body.push(match (&view.code, &view.error) {
        (Some(code), _) => text(crate::ui::elided_code(code))
            .size(11.0 * scale)
            .color(palette::TEXT),
        (None, None) => text("Код появится после выдачи.")
            .size(11.0 * scale)
            .color(palette::DIM),
        (None, Some(_)) => text("Код не выдан.").size(11.0 * scale).color(palette::ERR),
    });

    if view.code.is_some() {
        let copy = button(
            text("Скопировать код")
                .size(12.0 * scale)
                .font(palette::bold())
                .color(palette::BG),
        )
        .padding(Padding {
            top: 8.0,
            right: 14.0,
            bottom: 8.0,
            left: 14.0,
        })
        .style(palette::button_accent)
        .on_press(Message::Meet(MeetIn::CopyCode));

        let actions = match &view.qr {
            Some(data) => row![
                copy,
                space::Space::new().width(Length::Fill),
                qr_code::QRCode::new(data)
                    .total_size(120)
                    .style(palette::qr),
            ]
            .spacing(12)
            .align_y(iced::alignment::Vertical::Center),
            None => row![copy],
        };
        body = body.push(actions);
        body = body.push(
            text("Код одноразовый: действует до первого подключения.")
                .size(10.0 * scale)
                .color(palette::DIM),
        );
    }

    body = body.push(space::Space::new().height(4));
    body = body
        .push(
            text("Вставить чужой код")
                .size(11.0 * scale)
                .color(palette::DIM),
        )
        .push(
            row![
                text_input("Вставьте код приглашения", &view.foreign)
                    .on_input(|value| Message::Meet(MeetIn::ForeignInput(value)))
                    .on_submit(Message::Meet(MeetIn::SubmitForeign))
                    .size(13.0 * scale)
                    .padding(8)
                    .style(palette::input)
                    .width(Length::Fill),
                button(
                    text("Добавить")
                        .size(12.0 * scale)
                        .font(palette::bold())
                        .color(palette::BG),
                )
                .padding(Padding {
                    top: 8.0,
                    right: 14.0,
                    bottom: 8.0,
                    left: 14.0,
                })
                .style(palette::button_accent)
                .on_press_maybe(
                    (!view.submitting && !view.foreign.trim().is_empty())
                        .then_some(Message::Meet(MeetIn::SubmitForeign)),
                ),
            ]
            .spacing(8)
            .align_y(iced::alignment::Vertical::Center),
        );

    if let Some(error) = &view.error {
        body = body.push(text(error.clone()).size(11.0 * scale).color(palette::ERR));
    }

    container(body)
        .width(Length::Fixed(520.0))
        .padding(16)
        .style(palette::dialog)
        .into()
}

/// Карточка контакта: имя, состояние, блокировка (GUI_UI.md §1).
/// Отпечатка в API 1.0.0 нет (GET_IDENTITY — только свой) — карточка
/// показывает имя и действия.
pub fn contact_card(
    cid: Cid,
    name: &str,
    online: bool,
    blocked: bool,
    scale: f32,
) -> Element<'static, Message> {
    let (status, color) = if blocked {
        ("заблокирован", palette::ERR)
    } else if online {
        ("в сети", palette::OK)
    } else {
        ("не в сети", palette::DIM)
    };
    let block_label = if blocked {
        "Разблокировать"
    } else {
        "Заблокировать"
    };

    container(
        column![
            text(name.to_owned())
                .size(15.0 * scale)
                .font(palette::bold())
                .color(palette::TEXT),
            text(status).size(12.0 * scale).color(color),
            space::Space::new().height(4),
            row![
                button(text(block_label).size(12.0 * scale).color(if blocked {
                    palette::TEXT
                } else {
                    palette::ERR
                }))
                .padding(Padding {
                    top: 8.0,
                    right: 14.0,
                    bottom: 8.0,
                    left: 14.0,
                })
                .style(palette::button_dim)
                .on_press(Message::Meet(MeetIn::ToggleBlock(cid))),
                space::Space::new().width(Length::Fill),
                button(text("Закрыть").size(12.0 * scale).color(palette::TEXT))
                    .padding(Padding {
                        top: 8.0,
                        right: 14.0,
                        bottom: 8.0,
                        left: 14.0,
                    })
                    .style(palette::button_plain)
                    .on_press(Message::Meet(MeetIn::CloseOverlay)),
            ]
            .align_y(iced::alignment::Vertical::Center),
        ]
        .spacing(10),
    )
    .width(Length::Fixed(420.0))
    .padding(16)
    .style(palette::dialog)
    .into()
}

/// Меню по правой кнопке на контакте: блок/разблок и информация
/// (GUI_UI.md §4).
pub fn context_menu(
    menu: &MenuView,
    name: &str,
    blocked: bool,
    scale: f32,
) -> Element<'static, Message> {
    let label = if blocked {
        "Разблокировать"
    } else {
        "Заблокировать"
    };
    container(
        column![
            text(name.to_owned()).size(11.0 * scale).color(palette::DIM),
            button(text(label).size(13.0 * scale).color(if blocked {
                palette::TEXT
            } else {
                palette::ERR
            }))
            .width(Length::Fill)
            .padding(Padding {
                top: 6.0,
                right: 10.0,
                bottom: 6.0,
                left: 10.0,
            })
            .style(palette::button_menu)
            .on_press(Message::Meet(MeetIn::ToggleBlock(menu.cid))),
            button(text("Информация").size(13.0 * scale).color(palette::TEXT))
                .width(Length::Fill)
                .padding(Padding {
                    top: 6.0,
                    right: 10.0,
                    bottom: 6.0,
                    left: 10.0,
                })
                .style(palette::button_menu)
                .on_press(Message::Meet(MeetIn::OpenCard(menu.cid))),
        ]
        .spacing(2),
    )
    .width(Length::Fixed(180.0))
    .padding(8)
    .style(palette::dialog)
    .into()
}

/// Панель настроек: приватность уведомлений, компактная лента, масштаб
/// шрифта (S3, GUI_UI.md §4). Возвращает Element, заимствующий settings —
/// значения читаются из Store на каждый кадр.
pub fn settings_panel(settings: &Settings, scale: f32) -> Element<'_, Message> {
    use crate::model::settings::{FONT_SCALE_MAX, FONT_SCALE_MIN, FONT_SCALE_STEP};

    let percent = (settings.font_scale * 100.0).round() as i32;
    let step_button = |label: &'static str, delta: f32, enabled: bool| {
        button(text(label).size(12.0 * scale).color(palette::TEXT))
            .padding(Padding {
                top: 6.0,
                right: 14.0,
                bottom: 6.0,
                left: 14.0,
            })
            .style(palette::button_dim)
            .on_press_maybe(
                enabled.then_some(Message::Settings(SettingsIn::ChangeFontScale(delta))),
            )
    };

    container(
        column![
            text("Настройки")
                .size(14.0 * scale)
                .font(palette::bold())
                .color(palette::TEXT),
            iced::widget::checkbox(settings.hide_notification_text)
                .label("Скрывать текст в уведомлениях")
                .on_toggle(|on| Message::Settings(SettingsIn::ToggleNotificationText(on)))
                .text_size(12.0 * scale)
                .style(palette::checkbox_style),
            iced::widget::checkbox(settings.compact)
                .label("Компактная лента — без пузырей, всё слева")
                .on_toggle(|on| Message::Settings(SettingsIn::ToggleCompact(on)))
                .text_size(12.0 * scale)
                .style(palette::checkbox_style),
            text(format!("Масштаб шрифта: {percent}%"))
                .size(12.0 * scale)
                .color(palette::DIM),
            row![
                step_button(
                    "Уменьшить",
                    -FONT_SCALE_STEP,
                    settings.font_scale > FONT_SCALE_MIN
                ),
                step_button(
                    "Увеличить",
                    FONT_SCALE_STEP,
                    settings.font_scale < FONT_SCALE_MAX
                ),
            ]
            .spacing(8),
        ]
        .spacing(12),
    )
    .width(Length::Fixed(420.0))
    .padding(16)
    .style(palette::dialog)
    .into()
}
