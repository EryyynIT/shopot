//! Разметка сообщений (S14): ввод в стиле markdown → entities ядра
//! (CORE-API.md §4.3). Маркеры срезаются, offset/len — байты UTF-8
//! ИТОГОВОГО тела. Пары не вкладываются (внутри пары маркеры — текст);
//! незакрытый маркер остаётся литералом. Ядро правила перепроверяет
//! (BAD_ARGS), ui строит только валидные списки.

use crate::core_link::{Entity, EntityKind};

/// Маркеры: (открывающий, закрывающий, вид). `**` раньше `*` — жадность
/// по длине. Подчёркнутый `__`, зачёркнутый `~~`, спойлер `||`.
const PAIRS: &[(&str, &str, EntityKind)] = &[
    ("```", "```", EntityKind::CodeBlock),
    ("**", "**", EntityKind::Bold),
    ("__", "__", EntityKind::Underline),
    ("~~", "~~", EntityKind::Strike),
    ("||", "||", EntityKind::Spoiler),
    ("`", "`", EntityKind::Code),
    ("*", "*", EntityKind::Italic),
];

/// Разобрать ввод композера. Возвращает тело без маркеров и entities,
/// отсортированные по offset (правила §4.3). Текст без разметки проходит
/// без изменений.
pub fn parse(input: &str) -> (String, Vec<Entity>) {
    let mut body = String::with_capacity(input.len());
    let mut entities: Vec<Entity> = Vec::new();
    let mut rest = input;
    while !rest.is_empty() {
        // Ссылка [текст](url).
        if let Some(parsed) = parse_link(rest) {
            let (consumed, text, url) = parsed;
            entities.push(Entity::link(body.len() as u16, text.len() as u16, url));
            body.push_str(text);
            rest = &rest[consumed..];
            continue;
        }
        // Парные маркеры.
        if let Some((consumed, inner, kind, value)) = parse_pair(rest) {
            entities.push(Entity {
                kind,
                offset: body.len() as u16,
                len: inner.len() as u16,
                value,
            });
            body.push_str(inner);
            rest = &rest[consumed..];
            continue;
        }
        // Обычный текст: до ближайшего возможного маркера.
        let next = rest
            .char_indices()
            .skip(1)
            .find(|(_, c)| matches!(c, '*' | '_' | '~' | '|' | '`' | '['))
            .map(|(i, _)| i)
            .unwrap_or(rest.len());
        body.push_str(&rest[..next]);
        rest = &rest[next..];
    }
    (body, entities)
}

/// `[текст](url)` → (съедено байт, текст, url). Пустые части и
/// пробелы внутри скобок не считаются ссылкой.
fn parse_link(input: &str) -> Option<(usize, &str, String)> {
    if !input.starts_with('[') {
        return None;
    }
    let close = input.find("](")?;
    let text = &input[1..close];
    let after = &input[close + 2..];
    let end = after.find(')')?;
    let url = &after[..end];
    if text.is_empty() || url.is_empty() || url.len() > 255 {
        return None;
    }
    Some((close + 2 + end + 1, text, url.to_owned()))
}

/// Парный маркер в начале input → (съедено байт, содержимое, вид,
/// value). Содержимое непустое; блок кода может нести тег языка
/// (```` ```rust ````) — он уходит в value и не попадает в тело.
fn parse_pair(input: &str) -> Option<(usize, &str, EntityKind, String)> {
    for (open, close, kind) in PAIRS {
        if !input.starts_with(open) {
            continue;
        }
        let inner_start = open.len();
        let close_at = input[inner_start..].find(close)?;
        let mut inner = &input[inner_start..inner_start + close_at];
        let mut value = String::new();
        if *kind == EntityKind::CodeBlock {
            // ```lang\n код ``` — первая строка до \n: тег языка.
            if let Some(nl) = inner.find('\n') {
                let (tag, code) = inner.split_at(nl);
                let tag = tag.trim();
                if !tag.is_empty() {
                    value = tag.to_owned();
                }
                inner = &code[1..];
            }
        }
        if inner.is_empty() {
            return None;
        }
        return Some((inner_start + close_at + close.len(), inner, *kind, value));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Базовые виды: маркеры срезаны, offsets — байты итогового тела.
    #[test]
    fn all_kinds_parse_with_byte_offsets() {
        let (body, entities) = parse("а **жирный** и *курсив*");
        assert_eq!(body, "а жирный и курсив");
        // «а » = 3 байта; «жирный» = 12 байт (кириллица ×2).
        assert_eq!(entities[0], Entity::new(EntityKind::Bold, 3, 12));
        let italic_at = (body.find("курсив").unwrap()) as u16;
        assert_eq!(entities[1], Entity::new(EntityKind::Italic, italic_at, 12));
        assert_eq!(&body[3..3 + 12], "жирный");
    }

    /// Ссылки, спойлер, код, блок кода с языком.
    #[test]
    fn link_spoiler_code_and_block() {
        let (body, entities) = parse("смотри [шопот](https://shopot.dev) ||тайна|| `x=1`");
        assert_eq!(body, "смотри шопот тайна x=1");
        assert_eq!(entities[0].kind, EntityKind::Link);
        assert_eq!(entities[0].value, "https://shopot.dev");
        assert_eq!(
            &body[entities[0].offset as usize..][..entities[0].len as usize],
            "шопот"
        );
        assert_eq!(entities[1].kind, EntityKind::Spoiler);
        assert_eq!(entities[2].kind, EntityKind::Code);

        let (body, entities) = parse("```rust\nfn main() {}\n```");
        assert_eq!(body, "fn main() {}\n");
        assert_eq!(entities[0].kind, EntityKind::CodeBlock);
        assert_eq!(entities[0].value, "rust");
    }

    /// Незакрытый маркер — литерал; текст без разметки не меняется;
    /// порядок entities — по offset (правило §4.3).
    #[test]
    fn unmatched_markers_stay_literal() {
        assert_eq!(parse("просто текст").1, Vec::new());
        let (body, entities) = parse("звезда * одна");
        assert_eq!(body, "звезда * одна");
        assert!(entities.is_empty());

        let (_, entities) = parse("*а* потом **б** и *в*");
        let offsets: Vec<u16> = entities.iter().map(|e| e.offset).collect();
        let mut sorted = offsets.clone();
        sorted.sort_unstable();
        assert_eq!(offsets, sorted, "отсортированы по offset");
    }

    /// Результат проходит валидацию правил ядра.
    #[test]
    fn parse_output_passes_kernel_validation() {
        let inputs = [
            "**а** *б* __в__ ~~г~~ ||д|| `е` ```ж``` [з](https://x.y)",
            "много **разметки** подряд *везде*",
            "||спойлер с *маркером* внутри||", // внутри — литерал
        ];
        for input in inputs {
            let (body, entities) = parse(input);
            assert!(
                crate::core_link::entities_valid(&body, &entities).is_ok(),
                "{input} → {entities:?}"
            );
        }
    }
}
