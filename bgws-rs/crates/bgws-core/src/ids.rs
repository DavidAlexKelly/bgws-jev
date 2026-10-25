//! Newtype identifiers (§5.3). Never bare integers.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

macro_rules! id_type {
    ($(#[$m:meta])* $name:ident, $prefix:literal) => {
        $(#[$m])*
        #[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default)]
        #[serde(transparent)]
        pub struct $name(pub u32);

        impl $name {
            pub const fn new(n: u32) -> Self { Self(n) }
            pub const fn index(self) -> usize { self.0 as usize }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!($prefix, "{}"), self.0)
            }
        }
    };
}

id_type!(/// A platform, unit, munition or HQ in the world.
    EntityId, "e");
id_type!(/// A command post.
    HqId, "hq");
id_type!(/// A track in a side's track store.
    TrackId, "t");
id_type!(/// A message on the nets.
    MessageId, "m");
id_type!(/// A seat at an HQ: who decides for it.
    SeatId, "seat");

/// The two sides. Neutral covers civilians and third parties.
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Blue,
    Red,
    Neutral,
}

impl Side {
    pub const PLAYERS: [Side; 2] = [Side::Blue, Side::Red];

    pub fn enemy(self) -> Side {
        match self {
            Side::Blue => Side::Red,
            Side::Red => Side::Blue,
            Side::Neutral => Side::Neutral,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Side::Blue => "blue",
            Side::Red => "red",
            Side::Neutral => "neutral",
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }
}

impl std::str::FromStr for Side {
    type Err = String;
    fn from_str(s: &str) -> Result<Side, String> {
        match s.to_ascii_lowercase().as_str() {
            "blue" => Ok(Side::Blue),
            "red" => Ok(Side::Red),
            "neutral" => Ok(Side::Neutral),
            _ => Err(format!("unknown side `{s}` (blue, red, neutral)")),
        }
    }
}

impl fmt::Display for Side {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Stable string ids from the scenario, interned to dense numeric ids in
/// first-seen order. Lookup is by `BTreeMap`, so nothing depends on hash order.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Interner {
    names: Vec<String>,
    index: BTreeMap<String, u32>,
}

impl Interner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn intern(&mut self, name: &str) -> EntityId {
        if let Some(&i) = self.index.get(name) {
            return EntityId(i);
        }
        let i = self.names.len() as u32;
        self.names.push(name.to_owned());
        self.index.insert(name.to_owned(), i);
        EntityId(i)
    }

    pub fn get(&self, name: &str) -> Option<EntityId> {
        self.index.get(name).map(|&i| EntityId(i))
    }

    pub fn name(&self, id: EntityId) -> Option<&str> {
        self.names.get(id.index()).map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interning_is_stable_and_dense() {
        let mut i = Interner::new();
        let a = i.intern("blue-tank-1");
        let b = i.intern("red-ifv-2");
        assert_eq!(i.intern("blue-tank-1"), a);
        assert_eq!((a.0, b.0), (0, 1));
        assert_eq!(i.name(b), Some("red-ifv-2"));
        assert_eq!(Side::Blue.enemy(), Side::Red);
        assert_eq!(format!("{a}"), "e0");
    }
}
