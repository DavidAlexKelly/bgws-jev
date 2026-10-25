//! The event log: everything that happens, typed, in order, for the feed,
//! the AAR and analysis (§14.3). The fingerprint hashes the log, so two runs
//! can be compared cheaply (§12).

use crate::{geo::LatLon, ids::Owner, time::SimTime};
use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Domain {
    Land,
    Fires,
    Engineer,
    C2,
    Comms,
    Drones,
    Ew,
    Pnt,
    Air,
    AirDefence,
    Naval,
    Space,
    Cyber,
    Logistics,
    Medical,
    Killweb,
    People,
    Decision,
    Game,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LogEvent {
    pub t: SimTime,
    pub side: Owner,
    pub domain: Domain,
    /// A short machine kind: "detect", "hit", "kill", "msgLost", …
    pub kind: String,
    /// One line in words, for the feed.
    pub text: String,
    /// Who it is about, by scenario id.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<LatLon>,
    /// Why: the figures and factors behind it, for the AAR (design rule 10).
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub why: serde_json::Value,
}

/// FNV-1a over the canonical JSON of each event, in order.
#[derive(Clone, Debug)]
pub struct Fingerprint {
    h: u64,
    pub count: u64,
}

impl Default for Fingerprint {
    fn default() -> Self {
        Fingerprint { h: 0xcbf2_9ce4_8422_2325, count: 0 }
    }
}

impl Fingerprint {
    pub fn add(&mut self, e: &LogEvent) {
        let bytes = serde_json::to_vec(e).expect("events serialise");
        for b in bytes {
            self.h ^= b as u64;
            self.h = self.h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        self.count += 1;
    }
    pub fn hex(&self) -> String {
        format!("{:016x}", self.h)
    }
}
