//! The event queue (§6.2): a binary heap keyed (time, priority class, stable
//! key). Systems schedule follow-ups (a reload completes, a message arrives, a
//! pass begins) instead of polling. Ties never depend on insertion order or
//! hash order: the stable key comes from the event itself.

use bgws_core::{EntityId, SimTime};
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Which kind of thing happens first when two events share a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Priority {
    /// Scenario script events (reinforcements, weather changes, political switches).
    Scenario = 0,
    /// Physical arrivals: a munition lands, a convoy reaches a node.
    Physical = 1,
    /// Messages delivered over the nets.
    Comms = 2,
    /// Process completions: reloads, repairs, staff products.
    Process = 3,
    /// Decisions taking effect after their reaction time.
    Decision = 4,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Scheduled {
    /// A decision's answer takes effect for an agent.
    ApplyAnswer { question: u64 },
    /// A weapon finishes reloading.
    Reloaded { entity: EntityId, mount: u32 },
    /// A message arrives (delivery is computed by the comms model).
    MessageArrives { message: u64 },
    /// An indirect-fire mission's rounds land.
    RoundsLand { mission: u64 },
    /// A satellite pass starts or ends over an area.
    SatellitePass { satellite: EntityId, rising: bool },
    /// A scripted scenario event by index.
    ScriptEvent { index: u32 },
    /// A cyber effect window opens or closes.
    CyberWindow { campaign: u32, open: bool },
    /// A logistics delivery arrives at a node.
    Delivery { flow: u64 },
    /// An HQ's battle-rhythm event or plan completes.
    HqProduct { hq: EntityId, product: u64 },
    /// A generic timer for a system, keyed by name.
    Timer { system: String, key: u64 },
}

impl Scheduled {
    /// A stable key for tie-breaks, from the event's own contents.
    fn stable_key(&self) -> (u8, u64, u64) {
        match self {
            Scheduled::ApplyAnswer { question } => (0, *question, 0),
            Scheduled::Reloaded { entity, mount } => (1, entity.0 as u64, *mount as u64),
            Scheduled::MessageArrives { message } => (2, *message, 0),
            Scheduled::RoundsLand { mission } => (3, *mission, 0),
            Scheduled::SatellitePass { satellite, rising } => (4, satellite.0 as u64, *rising as u64),
            Scheduled::ScriptEvent { index } => (5, *index as u64, 0),
            Scheduled::CyberWindow { campaign, open } => (6, *campaign as u64, *open as u64),
            Scheduled::Delivery { flow } => (7, *flow, 0),
            Scheduled::HqProduct { hq, product } => (8, hq.0 as u64, *product),
            Scheduled::Timer { system, key } => (9, bgws_core::rng::fnv1a(system.as_bytes()), *key),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Entry {
    at: SimTime,
    priority: Priority,
    key: (u8, u64, u64),
    /// Only to tell apart two otherwise identical events; equal entries are
    /// interchangeable, so this never changes an outcome.
    n: u64,
    event: Scheduled,
}

impl Eq for Entry {}
impl PartialOrd for Entry {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Entry {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        (self.at, self.priority, self.key, self.n).cmp(&(o.at, o.priority, o.key, o.n))
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EventQueue {
    heap: BinaryHeap<Reverse<Entry>>,
    n: u64,
}

impl EventQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn schedule(&mut self, at: SimTime, priority: Priority, event: Scheduled) {
        let key = event.stable_key();
        self.n += 1;
        self.heap.push(Reverse(Entry { at, priority, key, n: self.n, event }));
    }

    pub fn next_time(&self) -> Option<SimTime> {
        self.heap.peek().map(|r| r.0.at)
    }

    /// Pop every event due at or before `t`, in order.
    pub fn pop_due(&mut self, t: SimTime) -> Vec<(SimTime, Scheduled)> {
        let mut out = Vec::new();
        while let Some(Reverse(e)) = self.heap.peek() {
            if e.at > t {
                break;
            }
            let Reverse(e) = self.heap.pop().expect("peeked");
            out.push((e.at, e.event));
        }
        out
    }

    pub fn len(&self) -> usize {
        self.heap.len()
    }

    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orders_by_time_then_priority_then_content() {
        let mut q = EventQueue::new();
        let t = SimTime::from_secs_i(5);
        q.schedule(t, Priority::Decision, Scheduled::ApplyAnswer { question: 1 });
        q.schedule(t, Priority::Physical, Scheduled::RoundsLand { mission: 9 });
        q.schedule(t, Priority::Physical, Scheduled::RoundsLand { mission: 2 });
        q.schedule(SimTime::from_secs_i(1), Priority::Decision, Scheduled::ApplyAnswer { question: 7 });
        let got = q.pop_due(t);
        assert_eq!(
            got.into_iter().map(|(_, e)| e).collect::<Vec<_>>(),
            vec![
                Scheduled::ApplyAnswer { question: 7 },
                Scheduled::RoundsLand { mission: 2 },
                Scheduled::RoundsLand { mission: 9 },
                Scheduled::ApplyAnswer { question: 1 },
            ]
        );
    }
}
