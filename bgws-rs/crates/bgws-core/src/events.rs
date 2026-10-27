//! The event log and its fingerprint (§12, §14.3). Every system writes typed
//! events here; the host drains them to the store, the UI feed and the AAR.
//! The fingerprint is a rolling hash over every event in order, so two runs
//! agree exactly when their fingerprints do.

use crate::ids::{EntityId, Side};
use crate::rng::{fnv1a, mix64};
use crate::time::SimTime;
use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LogEvent {
    pub t: SimTime,
    pub seq: u64,
    /// Dotted kind: `sense.detect`, `fire.hit`, `comms.delivered`, `decide.answer`.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side: Option<Side>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity: Option<EntityId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<EntityId>,
    /// One line for people.
    pub text: String,
    /// The figures and values behind the event (rule 10: explainable).
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub data: serde_json::Value,
}

#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export, type = "number"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct Fingerprint(pub u64);

impl std::fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EventLog {
    pending: Vec<LogEvent>,
    seq: u64,
    fingerprint: Fingerprint,
}

/// A builder so systems can write `log.push(t, "fire.hit").entity(a).target(b).text(..).data(..)`.
pub struct EventBuilder<'a> {
    log: &'a mut EventLog,
    ev: LogEvent,
}

impl<'a> EventBuilder<'a> {
    pub fn side(mut self, s: Side) -> Self {
        self.ev.side = Some(s);
        self
    }
    pub fn entity(mut self, e: EntityId) -> Self {
        self.ev.entity = Some(e);
        self
    }
    pub fn target(mut self, e: EntityId) -> Self {
        self.ev.target = Some(e);
        self
    }
    pub fn text(mut self, t: impl Into<String>) -> Self {
        self.ev.text = t.into();
        self
    }
    pub fn data(mut self, d: serde_json::Value) -> Self {
        self.ev.data = d;
        self
    }
}

impl Drop for EventBuilder<'_> {
    fn drop(&mut self) {
        let ev = std::mem::replace(
            &mut self.ev,
            LogEvent { t: SimTime::ZERO, seq: 0, kind: String::new(), side: None, entity: None, target: None, text: String::new(), data: serde_json::Value::Null },
        );
        self.log.record(ev);
    }
}

impl EventLog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, t: SimTime, kind: impl Into<String>) -> EventBuilder<'_> {
        EventBuilder {
            ev: LogEvent { t, seq: 0, kind: kind.into(), side: None, entity: None, target: None, text: String::new(), data: serde_json::Value::Null },
            log: self,
        }
    }

    pub fn record(&mut self, mut ev: LogEvent) {
        ev.seq = self.seq;
        self.seq += 1;
        let bytes = serde_json::to_vec(&ev).expect("events serialise");
        self.fingerprint = Fingerprint(mix64(self.fingerprint.0 ^ fnv1a(&bytes)));
        self.pending.push(ev);
    }

    pub fn drain(&mut self) -> Vec<LogEvent> {
        std::mem::take(&mut self.pending)
    }

    pub fn pending(&self) -> &[LogEvent] {
        &self.pending
    }

    pub fn fingerprint(&self) -> Fingerprint {
        self.fingerprint
    }

    pub fn count(&self) -> u64 {
        self.seq
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_tracks_content_and_order() {
        let mut a = EventLog::new();
        a.push(SimTime::ZERO, "x").text("one");
        a.push(SimTime::ZERO, "y").text("two");
        let mut b = EventLog::new();
        b.push(SimTime::ZERO, "y").text("two");
        b.push(SimTime::ZERO, "x").text("one");
        assert_ne!(a.fingerprint(), b.fingerprint());
        assert_eq!(a.drain().len(), 2);
        assert!(a.pending().is_empty());
        assert_eq!(a.count(), 2);
    }
}
