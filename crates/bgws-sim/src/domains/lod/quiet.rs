//! Quiet skipping (§6.3): when there is no contact, nothing in flight and no
//! question or report waiting, the clock may run over the stretch where no
//! sensor can detect anything without doing the land sensing pass.
//!
//! The horizon is the earliest time any land observer could have an enemy
//! land unit inside its sensor reach: for every observer–target pair, the
//! gap between their distance and the observer's reach (plus a margin),
//! divided by their fastest possible closing speed (each one's top speed,
//! times a safety factor), capped by the next scheduled event, the longest
//! jump (`sim.quiet.max_jump_s`) and the time limit. Before the horizon the
//! sensing pass has no candidate pair, so skipping it changes nothing: every
//! other system still runs at its rate, which is why a skipped run has the
//! same event log, and so the same fingerprint, as a stepped one (tested).
//!
//! Positions are carried over the gap by the same movers the stepped clock
//! uses rather than by a closed-form jump: speeds depend on the going under
//! each unit second by second, so an analytic jump could not promise the same
//! arrivals. What the host gains is the sensing pass (the bulk of a tick) and
//! the knowledge, from [`next_interesting_time`], of how far it may run the
//! clock flat out.

use super::{with_state, LodParams, LodState};
use crate::components::*;
use crate::sim::Sim;
use crate::systems::{alive, mover_class};
use bgws_catalogue::{Domain, SensorKind as CatSensor};
use bgws_core::{Enu, Side, SimDuration, SimTime};

/// Counters and the cached horizon.
#[derive(Clone, Debug, Default)]
pub struct QuietState {
    pub horizon: Option<SimTime>,
    /// Entities when the horizon was computed (a spawn invalidates it).
    pub order_len: usize,
    /// Don't recompute a horizon that came out too short before this.
    pub retry_at: SimTime,
    /// Base ticks run without the sensing pass, and horizons computed.
    pub skipped_ticks: u64,
    pub horizons: u64,
}

/// No contact, nothing in flight, no question, trigger or report waiting.
pub fn quiet_now(s: &Sim) -> bool {
    if s.ctx.over.is_some() || !s.ctx.triggers.is_empty() || !s.ctx.reports.is_empty() || s.ctx.pending.values().any(|p| !p.answered) {
        return false;
    }
    if crate::world::fast_active(s) || crate::world::air_active(s) {
        return false;
    }
    s.ctx.order.iter().all(|&e| {
        s.world.get::<Aware>(e).map(|a| a.seen.is_empty() && a.suspects.is_empty()).unwrap_or(true) && !matches!(s.world.get::<Act>(e).map(|a| &a.activity), Some(Activity::Engage { .. }))
    })
}

/// One land unit for the horizon.
struct Probe {
    side: Side,
    pos: Enu,
    /// Sensor reach plus the margin; negative when it does not observe.
    reach: f64,
    /// Fastest it could move, m/s.
    v: f64,
}

/// The earliest time any land observer could have an enemy inside its reach.
pub fn detection_horizon(s: &Sim, p: &LodParams) -> SimTime {
    let now = s.ctx.now;
    let max_r = s.ctx.p.sense_max_m;
    let probes: Vec<Probe> = s
        .ctx
        .order
        .iter()
        .filter(|&&e| alive(s, e))
        .filter_map(|&e| {
            let i = s.world.get::<Ident>(e)?;
            let k = s.world.get::<Kin>(e)?;
            if i.domain != Domain::Land {
                return None;
            }
            let pl = &i.platform;
            // The same sensors and cap as `systems::sensing`.
            let reach = pl.sensors.iter().filter(|x| matches!(x.kind, CatSensor::Eye | CatSensor::Optic | CatSensor::Thermal | CatSensor::ImageIntensifier | CatSensor::Eo)).map(|x| x.range_max_m).fold(f64::NEG_INFINITY, f64::max);
            let reach = if i.dummy || !reach.is_finite() { -1.0 } else { reach.min(max_r) + p.reach_margin_m };
            let v = if mover_class(pl.mover.kind).1 { pl.mover.road_kmh.max(pl.mover.cross_country_kmh).max(pl.mover.max_kmh) / 3.6 } else { 0.0 };
            Some(Probe { side: i.side, pos: k.pos, reach, v })
        })
        .collect();
    // Per observer: the nearest unit of another side, closing at the
    // observer's top speed plus the fastest of theirs (a safe bound, and one
    // squared distance per pair).
    let mut best = f64::INFINITY;
    for side in Side::PLAYERS.iter().chain([&Side::Neutral]) {
        let others: Vec<(f64, f64)> = probes.iter().filter(|t| t.side != *side).map(|t| (t.pos.e, t.pos.n)).collect();
        let v_other = probes.iter().filter(|t| t.side != *side).map(|t| t.v).fold(0.0, f64::max);
        for o in probes.iter().filter(|o| o.side == *side && o.reach >= 0.0) {
            let d2 = others.iter().map(|&(e, n)| (e - o.pos.e) * (e - o.pos.e) + (n - o.pos.n) * (n - o.pos.n)).fold(f64::INFINITY, f64::min);
            let gap = d2.sqrt() - o.reach;
            if gap <= 0.0 {
                return now;
            }
            let closing = (o.v + v_other) * p.closing_margin;
            if closing > 0.0 {
                best = best.min(gap / closing);
            }
        }
    }
    if best.is_finite() {
        // Whole microseconds, rounded down.
        now + SimDuration::from_secs(best)
    } else {
        SimTime(i64::MAX / 4)
    }
}

/// Quiet now? Then the next time something could happen (§6.3): the next
/// scheduled event or the earliest possible detection, capped by the longest
/// jump and the time limit. `None` when there is contact, something in
/// flight, or a question or report waiting.
pub fn next_interesting_time(s: &Sim) -> Option<SimTime> {
    if !quiet_now(s) {
        return None;
    }
    let p = s.world.get_resource::<LodState>().map(|st| st.p.clone()).unwrap_or_else(|| LodParams::from_figures(&s.ctx.figures));
    let now = s.ctx.now;
    let mut t = detection_horizon(s, &p);
    if let Some(q) = s.ctx.queue.next_time() {
        t = t.min(q);
    }
    t = t.min(now + SimDuration::from_secs(p.max_jump_s)).min(s.ctx.duration_limit());
    Some(t.max(now))
}

/// May the base tick at `t` (the next stop) skip the land sensing pass?
/// Only while quiet, before the detection horizon (which scheduled events
/// cannot bring closer: they change orders, not top speeds; a spawn resets
/// it), and not on a tick where an event is due. Counts what it skips.
pub(crate) fn may_skip_sensing(s: &mut Sim, t: SimTime) -> bool {
    with_state(s, |s, st| {
        if !quiet_now(s) {
            st.quiet.horizon = None;
            return false;
        }
        if s.ctx.queue.next_time().map(|q| q <= t).unwrap_or(false) {
            return false;
        }
        if st.quiet.order_len != s.ctx.order.len() {
            st.quiet.horizon = None;
        }
        if let Some(h) = st.quiet.horizon {
            if t < h {
                st.quiet.skipped_ticks += 1;
                return true;
            }
        }
        if s.ctx.now < st.quiet.retry_at {
            return false;
        }
        let now = s.ctx.now;
        let h = Some(detection_horizon(s, &st.p).min(now + SimDuration::from_secs(st.p.max_jump_s)));
        st.quiet.horizons += 1;
        st.quiet.horizon = h;
        st.quiet.order_len = s.ctx.order.len();
        match h {
            Some(h) if t < h => {
                st.quiet.skipped_ticks += 1;
                true
            }
            _ => {
                st.quiet.retry_at = s.ctx.now + SimDuration::from_secs(10.0);
                false
            }
        }
    })
    .unwrap_or(false)
}

/// (ticks skipped, horizons computed) so far.
pub fn stats(s: &Sim) -> (u64, u64) {
    s.world.get_resource::<LodState>().map(|st| (st.quiet.skipped_ticks, st.quiet.horizons)).unwrap_or((0, 0))
}
