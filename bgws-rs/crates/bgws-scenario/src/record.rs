//! The game record (RUST_ENGINE.md §12, §14.3): "scenario, seed, build,
//! machine, every order and command, every model answer, plus snapshots".
//! With it a game replays exactly without calling Jev or Claude again —
//! answers come back as `AnswerSource::Recorded` — and can be scrubbed and
//! restarted from a snapshot.
//!
//! The plan names a zip with JSON and binary parts; this is one JSON
//! document (binary snapshots base64-encoded inside it), which is simpler,
//! diffable and sufficient until snapshots grow large.

use crate::v4::ScenarioV4;
use bgws_core::{Answer, AnswerSource, Fingerprint, QuestionId, Seed, Side, SimTime};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use thiserror::Error;

pub const RECORD_FORMAT: &str = "bgws.record";
pub const RECORD_VERSION: u32 = 1;

/// Which build made the record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct BuildInfo {
    /// A build id (a git hash, or the crate version when unknown).
    pub id: String,
    pub version: String,
    /// Fingerprint of the catalogue the run used (`Catalogue::fingerprint`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalogue: Option<u64>,
}

impl BuildInfo {
    /// This crate's version, with a caller-supplied id.
    pub fn this(id: impl Into<String>) -> Self {
        BuildInfo { id: id.into(), version: env!("CARGO_PKG_VERSION").into(), catalogue: None }
    }
}

/// Which machine made the record (§12: a replay on another GPU may differ).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MachineInfo {
    pub os: String,
    pub arch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threads: Option<u32>,
}

impl MachineInfo {
    /// The OS and architecture this binary was built for; the host fills CPU and GPU.
    pub fn compiled_for() -> Self {
        MachineInfo { os: std::env::consts::OS.into(), arch: std::env::consts::ARCH.into(), cpu: None, gpu: None, threads: None }
    }
}

/// An order or command a seat gave, and when it took effect.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordedCommand {
    pub at: SimTime,
    /// The seat or HQ that gave it.
    pub seat: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side: Option<Side>,
    /// The structured orders (C2SIM-shaped, owned by `bgws-sim`).
    pub orders: Value,
}

/// An answer, and when it was given.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordedAnswer {
    pub at: SimTime,
    pub answer: Answer,
}

/// A snapshot's bytes or JSON.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "encoding", content = "data", rename_all = "camelCase")]
pub enum SnapshotData {
    Json(Value),
    /// Binary (postcard/bincode), base64 in the document.
    Base64(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordedSnapshot {
    pub at: SimTime,
    pub snapshot: SnapshotData,
}

impl RecordedSnapshot {
    pub fn bytes(at: SimTime, bytes: &[u8]) -> Self {
        RecordedSnapshot { at, snapshot: SnapshotData::Base64(base64_encode(bytes)) }
    }
    pub fn json(at: SimTime, v: Value) -> Self {
        RecordedSnapshot { at, snapshot: SnapshotData::Json(v) }
    }
    /// The snapshot's bytes (JSON snapshots as their compact text).
    pub fn to_bytes(&self) -> Result<Vec<u8>, RecordError> {
        match &self.snapshot {
            SnapshotData::Base64(s) => base64_decode(s).ok_or(RecordError::Base64),
            SnapshotData::Json(v) => serde_json::to_vec(v).map_err(|e| RecordError::Json(e.to_string())),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Error)]
pub enum RecordError {
    #[error("{path}: {message}")]
    Io { path: String, message: String },
    #[error("not JSON or not a game record: {0}")]
    Json(String),
    #[error("not a game record (format `{0}`)")]
    Format(String),
    #[error("game record version {0} is not supported")]
    Version(String),
    #[error("a snapshot's base64 is corrupt")]
    Base64,
}

/// Everything needed to replay a game exactly.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameRecord {
    pub format: String,
    pub version: u32,
    pub scenario: ScenarioV4,
    pub seed: Seed,
    pub build: BuildInfo,
    pub machine: MachineInfo,
    /// Figure key → value, for every figure the run set away from its default.
    #[serde(default)]
    pub figure_overrides: BTreeMap<String, f64>,
    /// Every command and order, in the order given.
    #[serde(default)]
    pub commands: Vec<RecordedCommand>,
    /// Every answer, in the order given.
    #[serde(default)]
    pub answers: Vec<RecordedAnswer>,
    #[serde(default)]
    pub snapshots: Vec<RecordedSnapshot>,
    /// The event-log fingerprint at the end, to check a replay against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<Fingerprint>,
    /// The outcome, as the sim reported it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Value>,
}

impl GameRecord {
    pub fn new(scenario: ScenarioV4, seed: Seed, build: BuildInfo, machine: MachineInfo) -> Self {
        GameRecord {
            format: RECORD_FORMAT.into(),
            version: RECORD_VERSION,
            scenario,
            seed,
            build,
            machine,
            figure_overrides: BTreeMap::new(),
            commands: Vec::new(),
            answers: Vec::new(),
            snapshots: Vec::new(),
            fingerprint: None,
            outcome: None,
        }
    }

    pub fn push_command(&mut self, at: SimTime, seat: impl Into<String>, side: Option<Side>, orders: Value) {
        self.commands.push(RecordedCommand { at, seat: seat.into(), side, orders });
    }

    pub fn push_answer(&mut self, at: SimTime, answer: Answer) {
        self.answers.push(RecordedAnswer { at, answer });
    }

    pub fn push_snapshot(&mut self, s: RecordedSnapshot) {
        self.snapshots.push(s);
    }

    /// The recorded answer to a question, re-labelled as replayed (§12).
    pub fn replay_answer(&self, q: QuestionId) -> Option<Answer> {
        self.answers.iter().find(|a| a.answer.question == q).map(|a| {
            let mut ans = a.answer.clone();
            ans.source = AnswerSource::Recorded;
            ans
        })
    }

    /// Commands that took effect in `[from, to)`.
    pub fn commands_between(&self, from: SimTime, to: SimTime) -> impl Iterator<Item = &RecordedCommand> {
        self.commands.iter().filter(move |c| c.at >= from && c.at < to)
    }

    /// The latest snapshot at or before `t`, for scrubbing.
    pub fn snapshot_before(&self, t: SimTime) -> Option<&RecordedSnapshot> {
        self.snapshots.iter().filter(|s| s.at <= t).max_by_key(|s| s.at)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn from_json(text: &str) -> Result<GameRecord, RecordError> {
        let v: Value = serde_json::from_str(text).map_err(|e| RecordError::Json(e.to_string()))?;
        let f = v.get("format").and_then(Value::as_str).unwrap_or("").to_owned();
        if f != RECORD_FORMAT {
            return Err(RecordError::Format(f));
        }
        if v.get("version").and_then(Value::as_u64) != Some(RECORD_VERSION as u64) {
            return Err(RecordError::Version(v.get("version").map(|x| x.to_string()).unwrap_or_else(|| "none".into())));
        }
        serde_json::from_value(v).map_err(|e| RecordError::Json(e.to_string()))
    }

    pub fn save(&self, path: &Path) -> Result<(), RecordError> {
        std::fs::write(path, self.to_json() + "\n").map_err(|e| RecordError::Io { path: path.display().to_string(), message: e.to_string() })
    }

    pub fn load(path: &Path) -> Result<GameRecord, RecordError> {
        let text = std::fs::read_to_string(path).map_err(|e| RecordError::Io { path: path.display().to_string(), message: e.to_string() })?;
        Self::from_json(&text)
    }
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding.
pub fn base64_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                s.push(B64[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

/// Decode standard base64 with padding; `None` if malformed.
pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    if b.len() % 4 != 0 {
        return None;
    }
    let val = |c: u8| -> Option<u32> { B64.iter().position(|&x| x == c).map(|p| p as u32) };
    let mut out = Vec::with_capacity(b.len() / 4 * 3);
    for (ci, chunk) in b.chunks(4).enumerate() {
        let last = ci == b.len() / 4 - 1;
        let pad = chunk.iter().rev().take_while(|&&c| c == b'=').count();
        if pad > 2 || (pad > 0 && !last) {
            return None;
        }
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            let v = if i >= 4 - pad { 0 } else { val(c)? };
            n |= v << (18 - 6 * i);
        }
        let bytes = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        out.extend_from_slice(&bytes[..3 - pad]);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn base64_known() {
        assert_eq!(base64_encode(b"Man"), "TWFu");
        assert_eq!(base64_encode(b"Ma"), "TWE=");
        assert_eq!(base64_encode(b"M"), "TQ==");
        assert_eq!(base64_decode("TWE=").unwrap(), b"Ma");
        assert!(base64_decode("T=E=").is_none());
        assert!(base64_decode("abc").is_none());
    }

    proptest! {
        #[test]
        fn base64_round_trips(v in proptest::collection::vec(any::<u8>(), 0..200)) {
            prop_assert_eq!(base64_decode(&base64_encode(&v)).unwrap(), v);
        }

        #[test]
        fn base64_decode_never_panics(s in ".{0,40}") {
            let _ = base64_decode(&s);
        }
    }
}
