//! Виджет-компоненты: не знают про протокол, читают только Store
//! и переданные данные (GUI_UI.md §2).

use iced::alignment::Horizontal;
use iced::widget::{button, column, container, qr_code, row, space, text, text_input};
use iced::{Element, Length, Padding};

use crate::model::cid::Cid;
use crate::model::feed::FeedItem;
use crate::model::msg::SendState;
use crate::ui::palette;
use crate::ui::{time_hm, ChatIn, InviteView, MeetIn, MenuView, Message};

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
) -> Element<'static, Message> {
    let bar = container(text(""))
        .width(Length::Fixed(3.0))
        .height(Length::Fill)
        .style(palette::chat_bar(selected));

    let dot = text(if online { "●" } else { "○" })
        .size(12)
        .color(if online { palette::OK } else { palette::DIM });

    let has_unread = unread > 0;
    // to_owned: фрагмент текста должен владеть строкой (Element 'static
    // для lazy), &str протянул бы заимствование из Store.
    let name = text(name.to_owned())
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
            container(text(unread.to_string()).size(11))
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
pub fn date_sep(ts_ms: i64) -> Element<'static, Message> {
    container(
        text(crate::ui::date_label(ts_ms))
            .size(11)
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
pub fn sys_line(body: String) -> Element<'static, Message> {
    container(text(body).size(11).color(palette::WARN))
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

/// Группа сообщений одного автора: пузыри моей стороны справа, чужие
/// слева; тексты в пузыре — по левому краю; время и статус — строкой
/// под пузырём, у его правой кромки. Fill-распорок в цепочке группы
/// нет ни одной: они протягивают ширину ближайшего ограничителя и
/// распахивают пузырь (правка по замерам пробы раскладки).
pub fn message_group(group: &FeedItem) -> Vec<Element<'static, Message>> {
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
    let wrap = |inner: iced::Element<'static, Message>| {
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
    };

    // Подвал: время + статус отправки (только у меня) — «часики у галочек».
    let status = if *mine {
        Some(match send {
            SendState::Sending => ("отправляю".to_owned(), palette::DIM),
            SendState::Sent => ("отправлено".to_owned(), palette::DIM),
            SendState::Queued => ("отправится позже".to_owned(), palette::DIM),
            SendState::Delivered => ("доставлено".to_owned(), palette::OK),
            SendState::Failed(reason) => (format!("не ушло: {reason}"), palette::ERR),
        })
    } else {
        None
    };
    let footer = || -> iced::Element<'static, Message> {
        let mut foot = row![text(time_hm(*ts_ms)).size(10).color(palette::MUTED)].spacing(4.0);
        if let Some((label, color)) = &status {
            foot = foot.push(text(label.clone()).size(10).color(*color));
        }
        foot.into()
    };

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
        let body_text = text(body.to_owned()).size(14).color(palette::TEXT);

        // Длинные тексты — без пузыря, на всю ширину (GUI_STYLE.md §2).
        if body.len() > 300 {
            if let Some(col) = pending.take() {
                elements.push(wrap(bubble(col).into()));
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
                elements.push(wrap(group_with_footer(block, footer())));
            } else {
                elements.push(wrap(block.into()));
            }
        } else {
            let col = pending
                .take()
                .unwrap_or_else(|| iced::widget::Column::new().spacing(2));
            pending = Some(col.push(body_text));
        }
    }
    if let Some(col) = pending.take() {
        elements.push(wrap(group_with_footer(bubble(col), footer())));
    }

    elements
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
pub fn invite_panel(view: &InviteView) -> Element<'_, Message> {
    let mut body = column![text("Приглашение")
        .size(14)
        .font(palette::bold())
        .color(palette::TEXT),]
    .spacing(10);

    body = body.push(text("Мой код").size(11).color(palette::DIM));
    body = body.push(match (&view.code, &view.error) {
        (Some(code), _) => text(crate::ui::elided_code(code))
            .size(11)
            .color(palette::TEXT),
        (None, None) => text("Код выдаётся…").size(11).color(palette::DIM),
        (None, Some(_)) => text("Код не выдан.").size(11).color(palette::ERR),
    });

    if view.code.is_some() {
        let copy = button(
            text("Скопировать код")
                .size(12)
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
                .size(10)
                .color(palette::DIM),
        );
    }

    body = body.push(space::Space::new().height(4));
    body = body
        .push(text("Вставить чужой код").size(11).color(palette::DIM))
        .push(
            row![
                text_input("Вставьте код приглашения", &view.foreign)
                    .on_input(|value| Message::Meet(MeetIn::ForeignInput(value)))
                    .on_submit(Message::Meet(MeetIn::SubmitForeign))
                    .size(13)
                    .padding(8)
                    .style(palette::input)
                    .width(Length::Fill),
                button(
                    text("Добавить")
                        .size(12)
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
        body = body.push(text(error.clone()).size(11).color(palette::ERR));
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
                .size(15)
                .font(palette::bold())
                .color(palette::TEXT),
            text(status).size(12).color(color),
            space::Space::new().height(4),
            row![
                button(text(block_label).size(12).color(if blocked {
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
                button(text("Закрыть").size(12).color(palette::TEXT))
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
pub fn context_menu(menu: &MenuView, name: &str, blocked: bool) -> Element<'static, Message> {
    let label = if blocked {
        "Разблокировать"
    } else {
        "Заблокировать"
    };
    container(
        column![
            text(name.to_owned()).size(11).color(palette::DIM),
            button(
                text(label)
                    .size(13)
                    .color(if blocked { palette::TEXT } else { palette::ERR })
            )
            .width(Length::Fill)
            .padding(Padding {
                top: 6.0,
                right: 10.0,
                bottom: 6.0,
                left: 10.0,
            })
            .style(palette::button_menu)
            .on_press(Message::Meet(MeetIn::ToggleBlock(menu.cid))),
            button(text("Информация").size(13).color(palette::TEXT))
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
