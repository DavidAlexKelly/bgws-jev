//! Sides and owners.

use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Blue,
    Red,
}

impl Side {
    pub const BOTH: [Side; 2] = [Side::Blue, Side::Red];
    pub fn enemy(self) -> Side {
        match self {
            Side::Blue => Side::Red,
            Side::Red => Side::Blue,
        }
    }
    pub fn index(self) -> usize {
        self as usize
    }
    pub fn name(self) -> &'static str {
        match self {
            Side::Blue => "blue",
            Side::Red => "red",
        }
    }
}

/// Who an entity belongs to: a side, or nobody (civil traffic, neutral shipping).
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Owner {
    Blue,
    Red,
    Neutral,
}

impl Owner {
    pub fn side(self) -> Option<Side> {
        match self {
            Owner::Blue => Some(Side::Blue),
            Owner::Red => Some(Side::Red),
            Owner::Neutral => None,
        }
    }
}

impl From<Side> for Owner {
    fn from(s: Side) -> Owner {
        match s {
            Side::Blue => Owner::Blue,
            Side::Red => Owner::Red,
        }
    }
}
