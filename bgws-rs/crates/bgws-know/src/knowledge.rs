//! Who knows what (§9.2).
//!
//! - [`AgentKnowledge`]: a unit's own sightings plus the side tracks it has
//!   been told about, each with the time it heard. It knows what reached it,
//!   not the whole side picture. As in the TypeScript side picture
//!   (SIDE_PICTURE.md), a unit holds at most one version of each track: the
//!   newest it took. Its own sighting is preferred when it is as new.
//! - [`Cop`]: an HQ's common operational picture, fusing every report that
//!   reaches it with covariance intersection, so the same sighting relayed
//!   twice does not make the HQ falsely certain. Reports about the same track
//!   id are matched directly; reports from other stores (another HQ's
//!   tracks) are matched by a Mahalanobis gate.
//!
//! Everything here is built from [`TrackView`]s, which carry no truth.

use crate::classify::Domain;
use crate::filter::StateEstimate;
use crate::fusion::ci_estimates;
use crate::params::KnowParams;
use crate::stats::{chi2_quantile, Ellipse};
use crate::store::{ScanReport, TrackStore, TrackView};
use bgws_core::{EntityId, HqId, Side, SimTime, TrackId};
use nalgebra::{Matrix2, Vector2};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Where a piece of knowledge came from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum KnowledgeOrigin {
    /// The agent's own sensors.
    OwnSighting,
    /// A report that reached the agent.
    Told { heard_at: SimTime, via: Option<EntityId> },
    /// An HQ's fused picture.
    Cop { hq: HqId, reports: u32 },
}

/// A track as one agent knows it, predicted to the time of asking.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KnownTrack {
    pub track: TrackId,
    pub domain: crate::classify::Domain,
    pub origin: KnowledgeOrigin,
    /// When the underlying estimate was last updated by a sensor.
    pub seen_at: SimTime,
    /// The estimate predicted to the time of asking.
    pub estimate: StateEstimate,
    pub ellipse: Ellipse,
    pub class_top: Vec<(crate::classify::Hypothesis, f64)>,
    pub quality: u8,
    pub sources: Vec<EntityId>,
}

impl KnownTrack {
    fn from_view(v: &TrackView, origin: KnowledgeOrigin, now: SimTime, p: &KnowParams) -> KnownTrack {
        let estimate = v.estimate.predicted(now, p.domain(v.domain).accel_psd);
        let ellipse = Ellipse::from_cov(&estimate.horizontal_cov());
        KnownTrack {
            track: v.id,
            domain: v.domain,
            origin,
            seen_at: v.last_update,
            quality: p.quality(ellipse.semi_major),
            ellipse,
            estimate,
            class_top: v.belief.top_k(p.view_top_k),
            sources: v.sources.clone(),
        }
    }
}

/// A report a unit was told, and when it heard it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Heard {
    pub heard_at: SimTime,
    pub via: Option<EntityId>,
    pub report: TrackView,
}

/// One unit's knowledge.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentKnowledge {
    pub agent: EntityId,
    pub side: Side,
    /// Its own sightings: the side track as it stood right after this unit's detection updated it.
    pub own: BTreeMap<TrackId, TrackView>,
    /// What it has been told.
    pub told: BTreeMap<TrackId, Heard>,
}

impl AgentKnowledge {
    pub fn new(agent: EntityId, side: Side) -> Self {
        AgentKnowledge { agent, side, own: BTreeMap::new(), told: BTreeMap::new() }
    }

    /// Record this unit's own sighting of a track.
    pub fn record_sighting(&mut self, view: TrackView) {
        self.own.insert(view.id, view);
    }

    /// Take the tracks this unit's detections fed in a scan.
    pub fn absorb_scan(&mut self, scan: &ScanReport, store: &TrackStore, p: &KnowParams) {
        for id in scan.tracks_seen_by(self.agent) {
            if let Some(v) = store.view(id, p) {
                self.record_sighting(v);
            }
        }
    }

    /// A report reached this unit. Taken only if newer than the version it
    /// holds and newer than its own sighting; returns whether it was taken.
    pub fn hear(&mut self, report: TrackView, heard_at: SimTime, via: Option<EntityId>) -> bool {
        if report.side != self.side {
            return false;
        }
        let t = report.last_update;
        if self.own.get(&report.id).is_some_and(|o| o.last_update >= t) {
            return false;
        }
        if self.told.get(&report.id).is_some_and(|h| h.report.last_update >= t) {
            return false;
        }
        self.told.insert(report.id, Heard { heard_at, via, report });
        true
    }

    /// Forget sightings and reports older than the memory, or about tracks
    /// whose id is in `gone` (dropped or merged in the side's store, if the
    /// unit has been told so).
    pub fn forget(&mut self, now: SimTime, gone: &BTreeSet<TrackId>, p: &KnowParams) {
        let mem = p.agent_memory;
        self.own.retain(|id, v| now - v.last_update <= mem && !gone.contains(id));
        self.told.retain(|id, h| now - h.report.last_update <= mem && !gone.contains(id));
    }

    /// Everything this unit knows, predicted to `now`, in track id order.
    pub fn picture(&self, now: SimTime, p: &KnowParams) -> Vec<KnownTrack> {
        let ids: BTreeSet<TrackId> = self.own.keys().chain(self.told.keys()).copied().collect();
        ids.into_iter()
            .filter_map(|id| match (self.own.get(&id), self.told.get(&id)) {
                (Some(o), Some(h)) if h.report.last_update > o.last_update => Some(KnownTrack::from_view(
                    &h.report,
                    KnowledgeOrigin::Told { heard_at: h.heard_at, via: h.via },
                    now,
                    p,
                )),
                (Some(o), _) => Some(KnownTrack::from_view(o, KnowledgeOrigin::OwnSighting, now, p)),
                (None, Some(h)) => Some(KnownTrack::from_view(
                    &h.report,
                    KnowledgeOrigin::Told { heard_at: h.heard_at, via: h.via },
                    now,
                    p,
                )),
                (None, None) => None,
            })
            .collect()
    }
}

/// One entry of an HQ's COP.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CopEntry {
    /// The id of the first track reported for this object.
    pub id: TrackId,
    /// Every track id reported for it (several stores, or a merge).
    pub aliases: BTreeSet<TrackId>,
    /// The fused report.
    pub fused: TrackView,
    pub sources: BTreeSet<EntityId>,
    pub first_heard: SimTime,
    pub last_heard: SimTime,
    pub reports: u32,
}

/// What a report did to the COP (for the AAR).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CopUpdate {
    pub entry: TrackId,
    pub outcome: CopOutcome,
    /// CI weight on the COP's previous estimate, when fused.
    pub omega: Option<f64>,
    /// Gate distance when matched by gating.
    pub d2: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CopOutcome {
    /// A new object in the COP.
    New,
    /// A newer version of the same track replaced the old one.
    Replaced,
    /// Fused by covariance intersection.
    Fused,
    /// Older than what the COP holds and from the same track: ignored.
    Ignored,
}

/// An HQ's common operational picture.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Cop {
    pub hq: HqId,
    pub side: Side,
    pub entries: BTreeMap<TrackId, CopEntry>,
}

impl Cop {
    pub fn new(hq: HqId, side: Side) -> Self {
        Cop { hq, side, entries: BTreeMap::new() }
    }

    fn gate_match(&self, r: &TrackView, p: &KnowParams) -> Option<(TrackId, f64)> {
        let gate = chi2_quantile(p.cop_gate_prob, 2.0);
        let mut best: Option<(TrackId, f64)> = None;
        for (id, e) in &self.entries {
            if e.fused.domain != r.domain {
                continue;
            }
            let t = e.fused.estimate.time.max(r.estimate.time);
            let q = p.domain(r.domain).accel_psd;
            let (a, b) = (e.fused.estimate.predicted(t, q), r.estimate.predicted(t, q));
            let d = Vector2::new(a.x[0] - b.x[0], a.x[1] - b.x[1]);
            let s: Matrix2<f64> = a.horizontal_cov() + b.horizontal_cov();
            let Some(inv) = s.try_inverse() else { continue };
            let d2 = d.dot(&(inv * d));
            if d2 <= gate && best.is_none_or(|(_, b)| d2 < b) {
                best = Some((*id, d2));
            }
        }
        best
    }

    /// A track report reached the HQ from `from` at `received_at`.
    pub fn receive(&mut self, report: &TrackView, from: EntityId, received_at: SimTime, p: &KnowParams) -> CopUpdate {
        let by_alias = self.entries.iter().find(|(_, e)| e.aliases.contains(&report.id)).map(|(k, _)| *k);
        let (key, d2) = match by_alias {
            Some(k) => (Some(k), None),
            None => match self.gate_match(report, p) {
                Some((k, d2)) => (Some(k), Some(d2)),
                None => (None, None),
            },
        };
        let Some(key) = key else {
            self.entries.insert(
                report.id,
                CopEntry {
                    id: report.id,
                    aliases: [report.id].into_iter().collect(),
                    fused: report.clone(),
                    sources: [from].into_iter().chain(report.sources.iter().copied()).collect(),
                    first_heard: received_at,
                    last_heard: received_at,
                    reports: 1,
                },
            );
            return CopUpdate { entry: report.id, outcome: CopOutcome::New, omega: None, d2: None };
        };
        let Some(e) = self.entries.get_mut(&key) else {
            return CopUpdate { entry: key, outcome: CopOutcome::Ignored, omega: None, d2 };
        };
        e.aliases.insert(report.id);
        e.sources.insert(from);
        e.sources.extend(report.sources.iter().copied());
        e.last_heard = e.last_heard.max(received_at);
        e.reports += 1;
        let same_track = e.fused.id == report.id && e.aliases.len() == 1;
        if same_track && report.last_update > e.fused.last_update {
            // A later state of the same track already contains the earlier one.
            e.fused = report.clone();
            return CopUpdate { entry: key, outcome: CopOutcome::Replaced, omega: None, d2 };
        }
        if same_track && report.last_update < e.fused.last_update {
            return CopUpdate { entry: key, outcome: CopOutcome::Ignored, omega: None, d2 };
        }
        // Possibly correlated information: covariance intersection.
        let q = p.domain(report.domain).accel_psd;
        match ci_estimates(&e.fused.estimate, &report.estimate, q, p.ci_criterion) {
            Some((est, r)) => {
                let mut fused = e.fused.clone();
                fused.belief.pool(&report.belief, r.omega);
                fused.class_top = fused.belief.top_k(p.view_top_k);
                fused.best_level = fused.belief.best_level;
                fused.position = est.position();
                fused.velocity = est.velocity();
                fused.ellipse = Ellipse::from_cov(&est.horizontal_cov());
                fused.sigma_up = est.p[(2, 2)].max(0.0).sqrt();
                fused.quality = p.quality(fused.ellipse.semi_major);
                fused.last_update = fused.last_update.max(report.last_update);
                fused.first_seen = fused.first_seen.min(report.first_seen);
                fused.status = fused.status.max(report.status);
                let srcs: BTreeSet<EntityId> = fused.sources.iter().chain(report.sources.iter()).copied().collect();
                fused.sources = srcs.into_iter().collect();
                fused.estimate = est;
                e.fused = fused;
                CopUpdate { entry: key, outcome: CopOutcome::Fused, omega: Some(r.omega), d2 }
            }
            None => CopUpdate { entry: key, outcome: CopOutcome::Ignored, omega: None, d2 },
        }
    }

    /// Drop entries that have gone stale or too uncertain by `now`.
    pub fn prune(&mut self, now: SimTime, p: &KnowParams) -> Vec<TrackId> {
        let mut gone = vec![];
        for (id, e) in &self.entries {
            let dp = p.domain(e.fused.domain);
            let est = e.fused.estimate.predicted(now, dp.accel_psd);
            if now - e.fused.last_update > dp.max_coast || est.sigma_max() > dp.drop_sigma {
                gone.push(*id);
            }
        }
        for id in &gone {
            self.entries.remove(id);
        }
        gone
    }

    /// The COP predicted to `now`, in id order.
    pub fn picture(&self, now: SimTime, p: &KnowParams) -> Vec<KnownTrack> {
        self.entries
            .values()
            .map(|e| KnownTrack::from_view(&e.fused, KnowledgeOrigin::Cop { hq: self.hq, reports: e.reports }, now, p))
            .collect()
    }

    /// Entries in a domain.
    pub fn count(&self, domain: Domain) -> usize {
        self.entries.values().filter(|e| e.fused.domain == domain).count()
    }
}
