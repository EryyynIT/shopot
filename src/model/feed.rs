//! Группировка ленты: правило сканируемости из GUI_STYLE.md §2.
//!
//! Подряд идущие сообщения одного автора склеиваются; новая группа —
//! сменился автор, прошло > 5 минут или настал новый день. Sys-строки
//! и визитки всегда сами по себе.

use crate::model::cid::Cid;
use crate::model::msg::{Dir, Msg, MsgId, SendState};

/// Разрыв группы: 5 минут.
const GAP_MS: i64 = 5 * 60 * 1000;

/// Тело сообщения внутри группы ленты: текст + разметка (S14) +
/// реакции (S15) + файл (S10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedBody {
    pub id: MsgId,
    pub body: String,
    pub entities: Vec<crate::core_link::Entity>,
    pub reactions: Vec<crate::model::msg::Reaction>,
    pub file: Option<crate::model::msg::FileRef>,
}

#[cfg(test)]
impl FeedBody {
    /// Короткий конструктор для тестов группировки.
    pub(crate) fn test(id: MsgId, body: &str) -> Self {
        Self {
            id,
            body: body.to_owned(),
            entities: Vec::new(),
            reactions: Vec::new(),
            file: None,
        }
    }
}

/// Элемент ленты после группировки. Подписи («Сегодня», время) — забота UI.
#[derive(Clone, Debug, PartialEq)]
pub enum FeedItem {
    /// Разделитель дня; ts — любое время этого дня.
    DateSep { ts_ms: i64 },
    /// Служебная строка ядра.
    Sys { text: String },
    /// Визитка третьего лица (S13): имя из карточки; ts — для разрыва дня.
    Card { name: String, ts_ms: i64 },
    /// Группа одного автора: метка (имя + время) у первой строки.
    Group {
        mine: bool,
        /// Автор чужой группы; имя подставляет Store.
        author: Option<Cid>,
        name: String,
        ts_ms: i64,
        /// Тела сообщений группы.
        bodies: Vec<FeedBody>,
        /// Статус отправки — у моей группы, по последнему сообщению.
        send: SendState,
    },
}

/// Сгруппировать ленту. `day_of(ts_ms)` — индекс календарного дня
/// (локальная зона — забота вызывающего).
pub fn group(msgs: &[Msg], day_of: impl Fn(i64) -> i64) -> Vec<FeedItem> {
    let mut items: Vec<FeedItem> = Vec::with_capacity(msgs.len());
    let mut current_day: Option<i64> = None;
    let mut group: Option<GroupAcc> = None;

    for msg in msgs {
        let day = day_of(msg.ts_ms);
        if current_day != Some(day) {
            if let Some(acc) = group.take() {
                items.push(acc.into_item());
            }
            items.push(FeedItem::DateSep { ts_ms: msg.ts_ms });
            current_day = Some(day);
        }

        match msg.dir {
            Dir::Sys => {
                if let Some(acc) = group.take() {
                    items.push(acc.into_item());
                }
                items.push(FeedItem::Sys {
                    text: msg.body.clone(),
                });
            }
            Dir::Card => {
                if let Some(acc) = group.take() {
                    items.push(acc.into_item());
                }
                items.push(FeedItem::Card {
                    name: msg.body.clone(),
                    ts_ms: msg.ts_ms,
                });
            }
            Dir::Out | Dir::In(_) => {
                let mine = matches!(msg.dir, Dir::Out);
                let continue_group = match &group {
                    Some(acc) => {
                        acc.mine == mine
                            && msg.is_same_group_as(acc.first)
                            && msg.ts_ms - acc.last_ts <= GAP_MS
                    }
                    None => false,
                };
                if continue_group {
                    let acc = group.as_mut().expect("группа существует");
                    acc.last_ts = msg.ts_ms;
                    acc.bodies.push(FeedBody {
                        id: msg.id,
                        body: msg.body.clone(),
                        entities: msg.entities.clone(),
                        reactions: msg.reactions.clone(),
                        file: msg.file.clone(),
                    });
                    if mine {
                        acc.last_send = msg.send.clone();
                    }
                } else {
                    if let Some(acc) = group.take() {
                        items.push(acc.into_item());
                    }
                    group = Some(GroupAcc {
                        mine,
                        author: match msg.dir {
                            Dir::In(cid) => Some(cid),
                            _ => None,
                        },
                        name: String::new(),
                        first: msg,
                        last_ts: msg.ts_ms,
                        bodies: vec![FeedBody {
                            id: msg.id,
                            body: msg.body.clone(),
                            entities: msg.entities.clone(),
                            reactions: msg.reactions.clone(),
                            file: msg.file.clone(),
                        }],
                        last_send: msg.send.clone(),
                    });
                }
            }
        }
    }

    if let Some(acc) = group.take() {
        items.push(acc.into_item());
    }
    items
}

struct GroupAcc<'a> {
    mine: bool,
    author: Option<Cid>,
    name: String,
    first: &'a Msg,
    last_ts: i64,
    bodies: Vec<FeedBody>,
    last_send: SendState,
}

impl GroupAcc<'_> {
    fn into_item(self) -> FeedItem {
        FeedItem::Group {
            mine: self.mine,
            author: self.author,
            name: self.name,
            ts_ms: self.first.ts_ms,
            bodies: self.bodies,
            send: if self.mine {
                self.last_send
            } else {
                SendState::Sent
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::cid::Cid;
    use crate::model::msg::{Msg, MsgId, SendState};

    fn incoming(id: u64, ts: i64, body: &str) -> Msg {
        Msg::incoming(MsgId(id), Cid(1), ts, body.to_owned(), None, Vec::new())
    }

    fn outgoing(id: u64, ts: i64, body: &str) -> Msg {
        Msg::outgoing(MsgId(id), ts, body.to_owned(), Vec::new())
    }

    #[test]
    fn five_minute_gap_breaks_group() {
        let msgs = [
            incoming(1, 0, "раз"),
            incoming(2, 4 * 60 * 1000, "два"),
            // Разрыв между вторым и третьим — 5 минут и 1 мс.
            incoming(3, 9 * 60 * 1000 + 1, "три"),
        ];
        let items = group(&msgs, |_| 0);
        assert_eq!(items.len(), 3);
        let FeedItem::Group { bodies, .. } = &items[1] else {
            panic!()
        };
        assert_eq!(bodies.len(), 2);
        let FeedItem::Group { bodies, .. } = &items[2] else {
            panic!()
        };
        assert_eq!(bodies.len(), 1);
    }

    /// Визитка (S13) — отдельный элемент: рвёт группу как sys-строка.
    #[test]
    fn card_splits_groups() {
        let msgs = [
            incoming(1, 0, "раз"),
            Msg::card(1_000, "Вера".to_owned()),
            incoming(2, 2_000, "два"),
        ];
        let items = group(&msgs, |_| 0);
        assert!(matches!(items[1], FeedItem::Group { .. }));
        let FeedItem::Card { name, ts_ms } = &items[2] else {
            panic!("визитка")
        };
        assert_eq!(name, "Вера");
        assert_eq!(*ts_ms, 1_000);
        assert!(
            matches!(items[3], FeedItem::Group { .. }),
            "группа разорвана"
        );
    }

    #[test]
    fn sys_line_splits_groups() {
        let msgs = [
            incoming(1, 0, "раз"),
            Msg::sys(1, "сессия установлена".to_owned()),
            incoming(2, 2, "два"),
        ];
        let items = group(&msgs, |_| 0);
        assert_eq!(
            items,
            vec![
                FeedItem::DateSep { ts_ms: 0 },
                FeedItem::Group {
                    mine: false,
                    author: Some(Cid(1)),
                    name: String::new(),
                    ts_ms: 0,
                    bodies: vec![FeedBody::test(MsgId(1), "раз")],
                    send: SendState::Sent,
                },
                FeedItem::Sys {
                    text: "сессия установлена".into()
                },
                FeedItem::Group {
                    mine: false,
                    author: Some(Cid(1)),
                    name: String::new(),
                    ts_ms: 2,
                    bodies: vec![FeedBody::test(MsgId(2), "два")],
                    send: SendState::Sent,
                },
            ]
        );
    }

    #[test]
    fn my_group_keeps_last_send_state() {
        let msgs = [outgoing(1, 0, "раз"), outgoing(2, 1, "два")];
        let items = group(&msgs, |_| 0);
        let FeedItem::Group {
            send,
            bodies,
            author,
            ..
        } = &items[1]
        else {
            panic!()
        };
        assert_eq!(*send, SendState::Sending);
        assert_eq!(bodies.len(), 2);
        assert_eq!(*author, None);
    }
}
