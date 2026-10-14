//! Идентификатор контакта: единственный ключ во всей модели.

use std::fmt;

/// Ключ контакта. Имя — только отображение, ключом не является.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct Cid(pub i64);

impl fmt::Display for Cid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
