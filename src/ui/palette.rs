//! Палитра и стили темы: продолжение TUI, а не новый язык
//! (GUI_STYLE.md §0). Один акцент (cyan), семантика — только для
//! состояний. Никаких цветов мимо этого модуля.

use iced::border::Radius;
use iced::widget::{button, container, text, text_editor};
use iced::{Background, Border, Color, Theme};

/// Основной фон окна.
pub const BG: Color = Color::from_rgb8(0x14, 0x17, 0x1B);
/// Панели: сайдбар, шапки, статус-строка.
pub const PANEL: Color = Color::from_rgb8(0x1B, 0x1F, 0x24);
/// Поднятые поверхности: пузыри, выбранный чат, ховер.
pub const RAISE: Color = Color::from_rgb8(0x25, 0x2A, 0x31);
/// Границы и линии.
pub const LINE: Color = Color::from_rgb8(0x2A, 0x2F, 0x36);
/// Основной текст.
pub const TEXT: Color = Color::from_rgb8(0xCD, 0xD3, 0xDA);
/// Второстепенный текст: время, подсказки, служебное.
/// Приглушённый текст: подписи и метки времени внутри пузырей.
pub const MUTED: Color = Color::from_rgb8(0x9A, 0xA4, 0xB0);

/// Приглушённый текст: хинты, выключенные опции.
pub const DIM: Color = Color::from_rgb8(0x7D, 0x85, 0x8F);
/// Единственный акцент (тот же hue, что cyan в TUI).
pub const ACCENT: Color = Color::from_rgb8(0x4C, 0xC2, 0xFF);
/// Семантика: онлайн / успех.
pub const OK: Color = Color::from_rgb8(0x57, 0xC7, 0x85);
/// Семантика: предупреждение, звонок (позже).
pub const WARN: Color = Color::from_rgb8(0xD9, 0xB8, 0x5C);
/// Семантика: ошибка, блокировка.
pub const ERR: Color = Color::from_rgb8(0xE0, 0x6C, 0x75);

/// Полужирный шрифт (только имена и активные элементы).
pub fn bold() -> iced::Font {
    iced::Font {
        weight: iced::font::Weight::Bold,
        ..iced::Font::default()
    }
}

/// Фон всего окна.
pub fn root(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(BG)),
        ..Default::default()
    }
}

/// Панель (сайдбар, шапки, статус-строка) с тонкой нижней линией.
pub fn panel(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(PANEL)),
        border: Border {
            color: LINE,
            width: 1.0,
            radius: Radius::default(),
        },
        ..Default::default()
    }
}

/// Пузырь чужого сообщения.
pub fn bubble_theirs(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(Color::from_rgb8(0x2A, 0x30, 0x38))),
        border: Border {
            color: LINE,
            width: 1.0,
            radius: Radius::from(10.0),
        },
        ..Default::default()
    }
}

/// Пузырь моего сообщения: фон в полтона к акценту, без заливки акцентом.
pub fn bubble_mine(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(Color::from_rgb8(0x1D, 0x3A, 0x4C))),
        border: Border {
            color: LINE,
            width: 0.0,
            radius: Radius::from(10.0),
        },
        ..Default::default()
    }
}

/// Бейдж непрочитанных: акцентная плашка, тёмная цифра.
pub fn badge(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(ACCENT)),
        text_color: Some(BG),
        border: Border {
            color: LINE,
            width: 0.0,
            radius: Radius::from(9.0),
        },
        ..Default::default()
    }
}

/// Текст цвета `color`.
pub fn text_colored(color: Color) -> impl Fn(&Theme) -> text::Style {
    move |_| text::Style { color: Some(color) }
}

/// Кнопка-строка чата: прозрачная, ховер на полтона, выбранный — поднят.
pub fn chat_row(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let hovered = matches!(status, button::Status::Hovered);
        let background = if selected || hovered {
            Some(Background::Color(RAISE))
        } else {
            None
        };
        button::Style {
            background,
            text_color: TEXT,
            border: Border {
                color: LINE,
                width: 0.0,
                radius: Radius::from(6.0),
            },
            ..Default::default()
        }
    }
}

/// Акцентная полоса слева у выбранного чата (GUI_UI.md §1).
pub fn chat_bar(selected: bool) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: if selected {
            Some(Background::Color(ACCENT))
        } else {
            None
        },
        ..Default::default()
    }
}

/// Длинный текст без пузыря: без фона и рамки.
pub fn no_bubble(_: &Theme) -> container::Style {
    container::Style {
        background: None,
        ..Default::default()
    }
}

/// Поле ввода: фон в тон пузырей, скругление; фокус — акцентная рамка.
pub fn editor(_: &Theme, status: text_editor::Status) -> text_editor::Style {
    let focused = matches!(status, text_editor::Status::Focused { .. });
    text_editor::Style {
        background: Background::Color(Color::from_rgb8(0x2A, 0x30, 0x38)),
        border: Border {
            color: if focused { ACCENT } else { LINE },
            width: 1.0,
            radius: Radius::from(10.0),
        },
        placeholder: DIM,
        value: TEXT,
        selection: Color::from_rgba8(0x4C, 0xC2, 0xFF, 0.30),
    }
}

/// Главная кнопка действия: акцентная заливка, тёмный текст.
pub fn button_accent(_: &Theme, status: button::Status) -> button::Style {
    let (bg, fg) = match status {
        button::Status::Disabled => (RAISE, DIM),
        _ => (ACCENT, BG),
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: fg,
        border: Border {
            color: LINE,
            width: 0.0,
            radius: Radius::from(8.0),
        },
        ..Default::default()
    }
}
