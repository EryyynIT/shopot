//! Виджет-компоненты: не знают про протокол, читают только Store
//! и производные данные (GUI_UI.md §2).

use iced::alignment::Horizontal;
use iced::widget::{button, container, row, text};
use iced::{Element, Length, Padding};

use crate::model::cid::Cid;
use crate::model::feed::FeedItem;
use crate::model::msg::SendState;
use crate::ui::palette;
use crate::ui::{time_hm, ChatIn, Message};

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
