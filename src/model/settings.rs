//! Настройки клиента GUI. Живут в памяти сессии: на диск не пишем
//! ничего, кроме контента ядра — инвариант GUI_DEV.md §0 (перезапуск
//! сбрасывает настройки, сохранение — отложено).

/// Минимальный масштаб шрифта.
pub const FONT_SCALE_MIN: f32 = 0.8;
/// Максимальный масштаб шрифта (layout обязан держаться и здесь).
pub const FONT_SCALE_MAX: f32 = 1.5;
/// Шаг кнопок «Уменьшить/Увеличить».
pub const FONT_SCALE_STEP: f32 = 0.1;

/// Выбор пользователя, не проекция ядра (GUI_STYLE.md §5, §7).
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// Скрывать текст в уведомлениях ОС: показывать только имя —
    /// приватность на общем экране, включена по умолчанию.
    pub hide_notification_text: bool,
    /// Масштаб шрифта — множитель базовых размеров view.
    pub font_scale: f32,
    /// Компактная лента: без пузырей, всё слева (GUI_STYLE.md §2).
    pub compact: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hide_notification_text: true,
            font_scale: 1.0,
            compact: false,
        }
    }
}

impl Settings {
    /// Сдвинуть масштаб и удержать в пределах [`FONT_SCALE_MIN`]..[`FONT_SCALE_MAX`].
    pub fn change_font_scale(&mut self, delta: f32) {
        self.font_scale = (self.font_scale + delta).clamp(FONT_SCALE_MIN, FONT_SCALE_MAX);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_plan() {
        let settings = Settings::default();
        assert!(
            settings.hide_notification_text,
            "скрытие текста в уведомлениях включено по умолчанию"
        );
        assert_eq!(settings.font_scale, 1.0);
        assert!(!settings.compact);
    }

    #[test]
    fn font_scale_clamps_at_bounds() {
        let mut settings = Settings::default();
        settings.change_font_scale(-1.0);
        assert_eq!(settings.font_scale, FONT_SCALE_MIN);
        settings.change_font_scale(10.0);
        assert_eq!(settings.font_scale, FONT_SCALE_MAX);
        settings.change_font_scale(-0.05);
        assert!((settings.font_scale - (FONT_SCALE_MAX - 0.05)).abs() < 1e-6);
    }
}
