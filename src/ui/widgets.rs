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

/// Строка сайдбара: акцентная полоса выбранного, точка presence,
/// имя, бейдж непрочитанных. Владеет данными — строится внутри lazy.
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

    let mut cells = row![bar, dot, name].spacing(8);
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

    button(
        cells
            .padding(Padding {
                top: 6.0,
                right: 8.0,
                bottom: 6.0,
                left: 8.0,
            })
            .align_y(iced::alignment::Vertical::Center),
    )
    .on_press(Message::Chat(ChatIn::Select(cid)))
    .style(palette::chat_row(selected))
    .width(Length::Fill)
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

/// Служебная строка ядра: серым, мелко, по центру.
pub fn sys_line(body: String) -> Element<'static, Message> {
    container(text(body).size(11).color(palette::DIM))
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

/// Группа сообщений одного автора: метка (имя + время) у первой строки,
/// статус отправки — под моей группой.
pub fn message_group(group: &FeedItem) -> Vec<Element<'static, Message>> {
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

    let mut elements = Vec::with_capacity(bodies.len() + 2);

    let author = if *mine { "я".to_owned() } else { name.clone() };
    elements.push(
        container(
            text(format!("{author} {}", time_hm(*ts_ms)))
                .size(11)
                .font(palette::bold())
                .color(palette::DIM),
        )
        .width(Length::Fill)
        .align_x(if *mine {
            Horizontal::Right
        } else {
            Horizontal::Left
        })
        .padding(Padding {
            top: 6.0,
            right: 2.0,
            bottom: 0.0,
            left: 2.0,
        })
        .into(),
    );

    for (_, body) in bodies {
        // Длинные тексты — без пузыря, на всю ширину (GUI_STYLE.md §2).
        let long = body.len() > 300;
        let bubble_style: fn(&iced::Theme) -> iced::widget::container::Style = if long {
            palette::no_bubble
        } else if *mine {
            palette::bubble_mine
        } else {
            palette::bubble_theirs
        };
        elements.push(
            container(text(body.to_owned()).size(13).color(palette::TEXT))
                .max_width(560)
                .padding(Padding {
                    top: 4.0,
                    right: 10.0,
                    bottom: 4.0,
                    left: 10.0,
                })
                .style(bubble_style)
                .into(),
        );
    }

    if *mine {
        let (label, color) = match send {
            SendState::Sending => ("отправляю".to_owned(), palette::DIM),
            SendState::Sent => ("отправлено".to_owned(), palette::DIM),
            SendState::Failed(reason) => (format!("не ушло: {reason}"), palette::ERR),
        };
        elements.push(
            container(text(label).size(11).color(color))
                .width(Length::Fill)
                .align_x(Horizontal::Right)
                .padding(Padding {
                    top: 1.0,
                    right: 4.0,
                    bottom: 4.0,
                    left: 0.0,
                })
                .into(),
        );
    }

    elements
}
