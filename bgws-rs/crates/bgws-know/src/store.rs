//! A side's track store (§9.1; SIDE_PICTURE.md generalised): one record per
//! side of what its sensors have reported, as Kalman-filtered tracks.
//!
//! Each scan (the detections that share a time) is gated and assigned by
//! global nearest neighbour; unassigned position-like detections start
//! tentative tracks, unassigned bearings wait for a crossing bearing (DF);
//! tentative tracks are confirmed M-of-N; tracks whose states are
//! statistically indistinguishable merge; tracks that have grown too
//! uncertain, or gone unseen too long, drop. False tracks (spoofed
//! detections, clutter) and duplicate tracks (two sensors' detections of one
//! target in a scan) arise from these rules rather than from dice **[ref R2]**.
//!
//! **Truth never leaves through a view (rule 1, §17).** A [`Detection`]
//! carries the true entity only inside a [`TruthTag`], which has no public
//! accessor and does not implement `Serialize`. The store copies it into its
//! [`AuditLog`] and nowhere else; the log is reachable only through
//! [`TrackStore::audit`], meant for the umpire and the AAR. [`TrackView`]
//! has no field that could hold it, and the static check at the bottom of
//! this file fails to compile if `TruthTag` ever becomes serialisable.
//!
//! Deterministic (§12): tracks live in a `BTreeMap` by id, ids are allocated
//! from a counter and never reused, and every scan's detections are sorted
//! into a canonical order before association, so the order the simulation
//! hands them over in cannot change the result.

use crate::assoc::{gnn, Candidate};
use crate::classify::{ClassBelief, ClassEvidence, Domain, Hypothesis, IdLevel};
use crate::df::{crossing_angle, triangulate, BearingLine};
use crate::filter::{Kf, Measurement, MotionModel, StateEstimate};
use crate::fusion::covariance_intersection;
use crate::params::KnowParams;
use crate::stats::{chi2_quantile, Ellipse};
use bgws_core::{EntityId, Enu, Side, SimDuration, SimTime, TrackId};
use nalgebra::{DMatrix, DVector, Matrix3};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

/// The kind of sensor that made a detection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SensorKind {
    Visual,
    Thermal,
    Radar,
    Sar,
    /// Electronic support / direction finding.
    Esm,
    Acoustic,
    Sonar,
    /// A human report (patrol, HUMINT) entered as a detection.
    Report,
}

/// How the detected thing seems to be moving, which picks the filter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MotionHint {
    /// A ground position, emplacement or anchored ship: near-static filter.
    Static,
    /// A mover: constant velocity (2D on land and sea, 3D in the air).
    Moving,
    /// A manoeuvring mover: coordinated turn.
    Manoeuvring,
}

/// The true entity behind a detection, or none for a spoofed or false one.
/// Deliberately opaque: no public accessor, no `Serialize`. Only the store's
/// audit log can read it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TruthTag(Option<EntityId>);

impl TruthTag {
    /// A detection of a real entity.
    pub fn of(id: EntityId) -> Self {
        TruthTag(Some(id))
    }
    /// A spoofed, decoy-emitter or clutter detection: nothing is there.
    pub fn none() -> Self {
        TruthTag(None)
    }
    pub(crate) fn get(self) -> Option<EntityId> {
        self.0
    }
}

/// One sensor detection, as the simulation hands it to the side's store.
#[derive(Clone, Debug, PartialEq)]
pub struct Detection {
    pub time: SimTime,
    pub observer: EntityId,
    pub sensor: SensorKind,
    pub domain: Domain,
    pub motion: MotionHint,
    pub measurement: Measurement,
    pub class: ClassEvidence,
    /// For the audit only.
    pub truth: TruthTag,
}

/// Tentative until confirmed M-of-N.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackStatus {
    Tentative,
    Confirmed,
}

/// Why a track left the store.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DropReason {
    /// No update for longer than the domain's coast time.
    Stale,
    /// Its error ellipse grew past the domain's drop threshold.
    TooUncertain,
    /// Tentative, and not confirmed within N looks.
    NotConfirmed,
    /// Absorbed into another track.
    Merged,
}

#[derive(Clone, Debug)]
struct Track {
    id: TrackId,
    domain: Domain,
    kf: Kf,
    status: TrackStatus,
    /// Bit 0 is the latest look.
    history: u32,
    looks: u32,
    created: SimTime,
    last_update: SimTime,
    belief: ClassBelief,
    sources: BTreeSet<EntityId>,
    sensors: BTreeSet<SensorKind>,
    updates: u32,
}

/// What a decider may know about a track: the estimate, its uncertainty,
/// the class belief, quality and provenance. Never the truth.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrackView {
    pub id: TrackId,
    /// The side holding the track.
    pub side: Side,
    pub domain: Domain,
    pub status: TrackStatus,
    pub model: MotionModel,
    pub position: Enu,
    pub velocity: Enu,
    /// 1σ horizontal error ellipse.
    pub ellipse: Ellipse,
    /// 1σ altitude error, metres.
    pub sigma_up: f64,
    /// The most likely hypotheses, best first.
    pub class_top: Vec<(Hypothesis, f64)>,
    /// The full belief (for fusing at an HQ).
    pub belief: ClassBelief,
    /// The best look so far.
    pub best_level: IdLevel,
    /// Link-16-style quality, 0–15.
    pub quality: u8,
    pub first_seen: SimTime,
    pub last_update: SimTime,
    /// Time since the last update, at the time the view was taken.
    pub age: SimDuration,
    pub updates: u32,
    /// Own-side observers that contributed.
    pub sources: Vec<EntityId>,
    pub sensors: Vec<SensorKind>,
    /// The common 6D estimate, valid at `estimate.time`.
    pub estimate: StateEstimate,
}

/// One detection's contribution, for the AAR.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AuditContribution {
    pub time: SimTime,
    pub observer: EntityId,
    pub sensor: SensorKind,
    /// The entity really behind the detection; `None` for a spoof or clutter.
    pub truth: Option<EntityId>,
    /// The label the sensor reported.
    pub reported: Option<Hypothesis>,
}

/// The umpire's record of one track.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrackAudit {
    pub track: TrackId,
    pub created: SimTime,
    pub contributions: Vec<AuditContribution>,
    pub merged_from: Vec<TrackId>,
    pub merged_into: Option<TrackId>,
    pub dropped: Option<(SimTime, DropReason)>,
}

/// The umpire's and AAR's view of a store: which truth fed which track.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AuditLog {
    pub tracks: BTreeMap<TrackId, TrackAudit>,
    /// Bearings that waited for a partner and expired, with their truth.
    pub expired_bearings: Vec<AuditContribution>,
}

impl AuditLog {
    /// The entity most of the track's detections came from (ties: lowest
    /// id); `None` if most came from nothing (a false track).
    pub fn truth_of(&self, id: TrackId) -> Option<EntityId> {
        let a = self.tracks.get(&id)?;
        let mut counts: BTreeMap<Option<EntityId>, u32> = BTreeMap::new();
        for c in &a.contributions {
            *counts.entry(c.truth).or_default() += 1;
        }
        let mut best: Option<(Option<EntityId>, u32)> = None;
        for (k, n) in counts {
            if best.map_or(true, |(_, b)| n > b) {
                best = Some((k, n));
            }
        }
        best.and_then(|(k, _)| k)
    }

    /// A track built mostly from detections of nothing.
    pub fn is_false(&self, id: TrackId) -> bool {
        self.tracks.contains_key(&id) && self.truth_of(id).is_none()
    }

    /// Live tracks (not dropped or merged) grouped by the entity behind them;
    /// more than one track per entity is a duplicate.
    pub fn live_by_truth(&self) -> BTreeMap<EntityId, Vec<TrackId>> {
        let mut out: BTreeMap<EntityId, Vec<TrackId>> = BTreeMap::new();
        for (id, a) in &self.tracks {
            if a.dropped.is_none() && a.merged_into.is_none() {
                if let Some(t) = self.truth_of(*id) {
                    out.entry(t).or_default().push(*id);
                }
            }
        }
        out
    }
}

/// One detection's fate in a scan.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AssocRecord {
    pub observer: EntityId,
    pub sensor: SensorKind,
    pub track: Option<TrackId>,
    /// Mahalanobis distance squared to the track it updated (None if it started one or is waiting).
    pub d2: Option<f64>,
    pub outcome: AssocOutcome,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssocOutcome {
    Updated,
    NewTrack,
    /// A bearing waiting for a crossing bearing.
    DfPending,
    /// A bearing that completed a DF fix and started a track.
    DfFix,
}

/// Two tracks found to be one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MergeRecord {
    pub kept: TrackId,
    pub absorbed: TrackId,
    pub d2: f64,
}

/// What a scan did (explainable, rule 10). Contains no truth.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ScanReport {
    pub time: SimTime,
    pub detections: Vec<AssocRecord>,
    pub confirmed: Vec<TrackId>,
    pub merges: Vec<MergeRecord>,
    pub dropped: Vec<(TrackId, DropReason)>,
}

impl ScanReport {
    /// The tracks this observer's detections updated or started in this scan.
    pub fn tracks_seen_by(&self, observer: EntityId) -> Vec<TrackId> {
        let mut v: Vec<TrackId> =
            self.detections.iter().filter(|r| r.observer == observer).filter_map(|r| r.track).collect();
        v.sort();
        v.dedup();
        v
    }
}

/// A side's track store.
#[derive(Clone, Debug)]
pub struct TrackStore {
    side: Side,
    next_id: u32,
    time: SimTime,
    tracks: BTreeMap<TrackId, Track>,
    pending_bearings: Vec<Detection>,
    audit: AuditLog,
}

fn measurement_bits(m: &Measurement) -> Vec<u64> {
    match *m {
        Measurement::Position { pos, cov } => {
            let mut v = vec![0, pos.e.to_bits(), pos.n.to_bits(), pos.u.to_bits()];
            v.extend(cov.iter().map(|x| x.to_bits()));
            v
        }
        Measurement::RangeBearing { origin, range, bearing, sigma_range, sigma_bearing } => vec![
            1,
            origin.e.to_bits(),
            origin.n.to_bits(),
            origin.u.to_bits(),
            range.to_bits(),
            bearing.to_bits(),
            sigma_range.to_bits(),
            sigma_bearing.to_bits(),
        ],
        Measurement::Bearing { origin, bearing, sigma_bearing } => {
            vec![2, origin.e.to_bits(), origin.n.to_bits(), origin.u.to_bits(), bearing.to_bits(), sigma_bearing.to_bits()]
        }
    }
}

/// The canonical order of detections within a scan (truth plays no part).
fn canonical(a: &Detection, b: &Detection) -> Ordering {
    (a.time, a.observer, a.sensor, a.domain, a.motion)
        .cmp(&(b.time, b.observer, b.sensor, b.domain, b.motion))
        .then_with(|| measurement_bits(&a.measurement).cmp(&measurement_bits(&b.measurement)))
        .then_with(|| (a.class.level, a.class.reported, a.class.resolves_decoys).cmp(&(b.class.level, b.class.reported, b.class.resolves_decoys)))
}

fn model_for(domain: Domain, motion: MotionHint) -> MotionModel {
    match (motion, domain) {
        (MotionHint::Static, _) => MotionModel::Static,
        (MotionHint::Manoeuvring, _) => MotionModel::Ct,
        (MotionHint::Moving, Domain::Air) => MotionModel::Cv3,
        (MotionHint::Moving, _) => MotionModel::Cv2,
    }
}

fn contribution(d: &Detection) -> AuditContribution {
    AuditContribution { time: d.time, observer: d.observer, sensor: d.sensor, truth: d.truth.get(), reported: d.class.reported }
}

impl TrackStore {
    pub fn new(side: Side) -> Self {
        TrackStore { side, next_id: 1, time: SimTime::ZERO, tracks: BTreeMap::new(), pending_bearings: vec![], audit: AuditLog::default() }
    }

    pub fn side(&self) -> Side {
        self.side
    }

    /// The time the tracks are predicted to.
    pub fn time(&self) -> SimTime {
        self.time
    }

    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    /// Live track ids, in order.
    pub fn ids(&self) -> Vec<TrackId> {
        self.tracks.keys().copied().collect()
    }

    /// Bearings waiting for a DF partner.
    pub fn pending_bearings(&self) -> usize {
        self.pending_bearings.len()
    }

    /// **Umpire and AAR only.** Which truth fed which track. Decision code
    /// must never call this; it reads a [`crate::view::SideView`].
    pub fn audit(&self) -> &AuditLog {
        &self.audit
    }

    fn alloc(&mut self) -> TrackId {
        let id = TrackId(self.next_id);
        self.next_id += 1;
        id
    }

    /// Feed detections in any order: they are grouped by time into scans and
    /// each scan is processed in time order.
    pub fn ingest(&mut self, mut detections: Vec<Detection>, p: &KnowParams) -> Vec<ScanReport> {
        detections.sort_by(canonical);
        let mut out = vec![];
        let mut i = 0;
        while i < detections.len() {
            let t = detections[i].time;
            let j = detections[i..].iter().position(|d| d.time != t).map_or(detections.len(), |k| i + k);
            out.push(self.ingest_scan(t, detections[i..j].to_vec(), p));
            i = j;
        }
        out
    }

    /// Predict every track to `time` and drop those that have grown stale
    /// or too uncertain. Never goes backwards.
    pub fn predict_to(&mut self, time: SimTime, p: &KnowParams) -> Vec<(TrackId, DropReason)> {
        if time > self.time {
            self.time = time;
        }
        let mut dropped = vec![];
        for t in self.tracks.values_mut() {
            t.kf.predict_to(self.time, &p.process_noise(t.domain));
            let dp = p.domain(t.domain);
            if self.time - t.last_update > dp.max_coast {
                dropped.push((t.id, DropReason::Stale));
            } else if t.kf.estimate().sigma_max() > dp.drop_sigma {
                dropped.push((t.id, DropReason::TooUncertain));
            }
        }
        for &(id, why) in &dropped {
            self.drop_track(id, why);
        }
        dropped
    }

    fn drop_track(&mut self, id: TrackId, why: DropReason) {
        self.tracks.remove(&id);
        if let Some(a) = self.audit.tracks.get_mut(&id) {
            a.dropped = Some((self.time, why));
        }
    }

    fn start_track(&mut self, dets: &[&Detection], pos: Enu, cov: Matrix3<f64>, p: &KnowParams) -> TrackId {
        let first = dets[0];
        let id = self.alloc();
        let model = model_for(first.domain, first.motion);
        let kf = Kf::new(model, self.time, pos, cov, p.kf_init(first.domain, model == MotionModel::Static));
        let mut belief = p.confusion.prior(first.domain);
        for d in dets {
            belief.update(&p.confusion, &d.class);
        }
        let status = if p.confirm_m <= 1 { TrackStatus::Confirmed } else { TrackStatus::Tentative };
        self.tracks.insert(
            id,
            Track {
                id,
                domain: first.domain,
                kf,
                status,
                history: 1,
                looks: 1,
                created: self.time,
                last_update: self.time,
                belief,
                sources: dets.iter().map(|d| d.observer).collect(),
                sensors: dets.iter().map(|d| d.sensor).collect(),
                updates: dets.len() as u32,
            },
        );
        self.audit.tracks.insert(
            id,
            TrackAudit {
                track: id,
                created: self.time,
                contributions: dets.iter().map(|d| contribution(d)).collect(),
                merged_from: vec![],
                merged_into: None,
                dropped: None,
            },
        );
        id
    }

    /// Process one scan: every detection is taken as made at `time`.
    pub fn ingest_scan(&mut self, time: SimTime, mut dets: Vec<Detection>, p: &KnowParams) -> ScanReport {
        dets.sort_by(canonical);
        let mut report = ScanReport { time, ..Default::default() };
        report.dropped = self.predict_to(time, p);
        let gate = |dof: usize| chi2_quantile(p.gate_prob, dof as f64);
        let gates_by_dof = [0.0, gate(1), gate(2), gate(3)];

        // Gate every detection against every track of its domain.
        let ids = self.ids();
        let mut cands = vec![];
        for (i, d) in dets.iter().enumerate() {
            for (j, id) in ids.iter().enumerate() {
                let t = &self.tracks[id];
                if t.domain != d.domain {
                    continue;
                }
                if let Some(inn) = t.kf.innovation(&d.measurement) {
                    if inn.nis <= gates_by_dof[inn.dof.min(3)] {
                        cands.push(Candidate { detection: i, track: j, d2: inn.nis });
                    }
                }
            }
        }
        let unassigned_cost: Vec<f64> = dets
            .iter()
            .map(|d| gates_by_dof[match d.measurement {
                Measurement::Position { .. } => 3,
                Measurement::RangeBearing { .. } => 2,
                Measurement::Bearing { .. } => 1,
            }])
            .collect();
        let assignment = gnn(dets.len(), ids.len(), &cands, &unassigned_cost);

        // Updates.
        let mut hit_by: BTreeMap<TrackId, BTreeSet<EntityId>> = BTreeMap::new();
        let mut unassigned = vec![];
        for (i, d) in dets.iter().enumerate() {
            let Some(j) = assignment.of_detection[i] else {
                unassigned.push(i);
                continue;
            };
            let id = ids[j];
            let Some(t) = self.tracks.get_mut(&id) else { continue };
            let inn = t.kf.update(&d.measurement);
            t.belief.update(&p.confusion, &d.class);
            t.sources.insert(d.observer);
            t.sensors.insert(d.sensor);
            t.last_update = time;
            t.updates += 1;
            hit_by.entry(id).or_default().insert(d.observer);
            if let Some(a) = self.audit.tracks.get_mut(&id) {
                a.contributions.push(contribution(d));
            }
            report.detections.push(AssocRecord {
                observer: d.observer,
                sensor: d.sensor,
                track: Some(id),
                d2: inn.map(|x| x.nis),
                outcome: AssocOutcome::Updated,
            });
        }

        // Bearings waiting for a partner expire.
        let max_age = p.df_max_age;
        let (keep, expired): (Vec<Detection>, Vec<Detection>) =
            std::mem::take(&mut self.pending_bearings).into_iter().partition(|b| time - b.time <= max_age);
        self.pending_bearings = keep;
        self.audit.expired_bearings.extend(expired.iter().map(contribution));

        // New tracks and DF fixes.
        let mut created = BTreeSet::new();
        for i in unassigned {
            let d = &dets[i];
            match d.measurement {
                Measurement::Position { .. } | Measurement::RangeBearing { .. } => {
                    let u_default = match d.measurement {
                        Measurement::RangeBearing { origin, .. } => origin.u,
                        _ => 0.0,
                    };
                    let Some((pos, cov)) = d.measurement.as_position(u_default, p.unknown_alt_sigma.powi(2)) else { continue };
                    let id = self.start_track(&[d], pos, cov, p);
                    created.insert(id);
                    hit_by.entry(id).or_default().insert(d.observer);
                    report.detections.push(AssocRecord { observer: d.observer, sensor: d.sensor, track: Some(id), d2: None, outcome: AssocOutcome::NewTrack });
                }
                Measurement::Bearing { origin, bearing, sigma_bearing } => {
                    let this = BearingLine { origin, bearing, sigma: sigma_bearing };
                    let partners: Vec<usize> = (0..self.pending_bearings.len())
                        .filter(|&k| {
                            let b = &self.pending_bearings[k];
                            let Measurement::Bearing { origin: o, bearing: th, .. } = b.measurement else { return false };
                            b.domain == d.domain
                                && (o.e - origin.e).hypot(o.n - origin.n) > 1.0
                                && crossing_angle(th, bearing) >= p.df_min_crossing
                        })
                        .collect();
                    let mut lines = vec![this];
                    for &k in &partners {
                        if let Measurement::Bearing { origin, bearing, sigma_bearing } = self.pending_bearings[k].measurement {
                            lines.push(BearingLine { origin, bearing, sigma: sigma_bearing });
                        }
                    }
                    match triangulate(&lines, p.df_min_crossing, p.df_max_range) {
                        Some(fix) if !partners.is_empty() => {
                            let used: Vec<Detection> = partners.iter().map(|&k| self.pending_bearings[k].clone()).collect();
                            let mut k = 0;
                            self.pending_bearings.retain(|_| {
                                let keep = !partners.contains(&k);
                                k += 1;
                                keep
                            });
                            let mut cov = Matrix3::zeros();
                            cov.fixed_view_mut::<2, 2>(0, 0).copy_from(&fix.cov);
                            cov[(2, 2)] = p.unknown_alt_sigma.powi(2);
                            let pos = Enu::new(fix.pos.x, fix.pos.y, origin.u);
                            let mut all: Vec<&Detection> = vec![d];
                            all.extend(used.iter());
                            let id = self.start_track(&all, pos, cov, p);
                            created.insert(id);
                            for x in &all {
                                hit_by.entry(id).or_default().insert(x.observer);
                            }
                            report.detections.push(AssocRecord { observer: d.observer, sensor: d.sensor, track: Some(id), d2: None, outcome: AssocOutcome::DfFix });
                        }
                        _ => {
                            self.pending_bearings.push(d.clone());
                            report.detections.push(AssocRecord { observer: d.observer, sensor: d.sensor, track: None, d2: None, outcome: AssocOutcome::DfPending });
                        }
                    }
                }
            }
        }

        // M-of-N: a tentative track counts a look when a sensor that fed it
        // reported in this scan in its domain; a hit when it was updated.
        let looked: BTreeSet<(EntityId, Domain)> = dets.iter().map(|d| (d.observer, d.domain)).collect();
        let mask = if p.confirm_n >= 32 { u32::MAX } else { (1u32 << p.confirm_n.max(1)) - 1 };
        let mut not_confirmed = vec![];
        for t in self.tracks.values_mut() {
            if created.contains(&t.id) || t.status == TrackStatus::Confirmed {
                continue;
            }
            let hit = hit_by.contains_key(&t.id);
            let look = hit || t.sources.iter().any(|s| looked.contains(&(*s, t.domain)));
            if !look {
                continue;
            }
            t.history = ((t.history << 1) | hit as u32) & mask;
            t.looks += 1;
            if t.history.count_ones() >= p.confirm_m {
                t.status = TrackStatus::Confirmed;
                report.confirmed.push(t.id);
            } else if t.looks >= p.confirm_n {
                not_confirmed.push(t.id);
            }
        }
        for id in not_confirmed {
            self.drop_track(id, DropReason::NotConfirmed);
            report.dropped.push((id, DropReason::NotConfirmed));
        }

        report.merges = self.merge_pass(&hit_by, p);
        report
    }

    /// Merge tracks whose states are statistically indistinguishable. Two
    /// tracks the same observer updated in this scan are distinct objects
    /// and never merge.
    fn merge_pass(&mut self, hit_by: &BTreeMap<TrackId, BTreeSet<EntityId>>, p: &KnowParams) -> Vec<MergeRecord> {
        let threshold = chi2_quantile(p.merge_prob, 4.0);
        let mut merges = vec![];
        let ids = self.ids();
        for (a_i, &a) in ids.iter().enumerate() {
            for &b in &ids[a_i + 1..] {
                let (Some(ta), Some(tb)) = (self.tracks.get(&a), self.tracks.get(&b)) else { continue };
                if ta.domain != tb.domain {
                    continue;
                }
                if let (Some(oa), Some(ob)) = (hit_by.get(&a), hit_by.get(&b)) {
                    if !oa.is_disjoint(ob) {
                        continue;
                    }
                }
                let (ea, eb) = (ta.kf.estimate(), tb.kf.estimate());
                let idx = [0usize, 1, 3, 4];
                let d = DVector::from_iterator(4, idx.iter().map(|&i| ea.x[i] - eb.x[i]));
                let s = DMatrix::from_fn(4, 4, |r, c| ea.p[(idx[r], idx[c])] + eb.p[(idx[r], idx[c])]);
                let Some(ch) = s.cholesky() else { continue };
                let d2 = d.dot(&ch.solve(&d));
                if d2 < threshold {
                    self.absorb(a, b, p);
                    merges.push(MergeRecord { kept: a, absorbed: b, d2 });
                }
            }
        }
        merges
    }

    /// Fold track `b` into track `a` (the older id survives).
    fn absorb(&mut self, a: TrackId, b: TrackId, p: &KnowParams) {
        let Some(tb) = self.tracks.remove(&b) else { return };
        let prior = p.confusion.prior(tb.domain);
        if let Some(ta) = self.tracks.get_mut(&a) {
            if ta.kf.model == tb.kf.model {
                if let Some(r) = covariance_intersection(&ta.kf.x, &ta.kf.p, &tb.kf.x, &tb.kf.p, p.ci_criterion) {
                    ta.kf.x = r.x;
                    ta.kf.p = r.p;
                }
            } else if tb.kf.p.trace() / (tb.kf.model.dim() as f64) < ta.kf.p.trace() / (ta.kf.model.dim() as f64)
                && tb.kf.model != MotionModel::Static
            {
                // Different dynamics: keep the sharper mover's filter.
                ta.kf = tb.kf.clone();
            }
            ta.belief.merge_independent(&tb.belief, &prior);
            ta.sources.extend(tb.sources.iter().copied());
            ta.sensors.extend(tb.sensors.iter().copied());
            ta.history |= tb.history;
            ta.status = ta.status.max(tb.status);
            ta.created = ta.created.min(tb.created);
            ta.last_update = ta.last_update.max(tb.last_update);
            ta.updates += tb.updates;
        }
        let moved = self.audit.tracks.get_mut(&b).map(|ab| {
            ab.merged_into = Some(a);
            ab.dropped = Some((self.time, DropReason::Merged));
            ab.contributions.clone()
        });
        if let Some(aa) = self.audit.tracks.get_mut(&a) {
            aa.merged_from.push(b);
            aa.contributions.extend(moved.unwrap_or_default());
        }
    }

    fn view_of(&self, t: &Track, p: &KnowParams) -> TrackView {
        let estimate = t.kf.estimate();
        let ellipse = Ellipse::from_cov(&estimate.horizontal_cov());
        TrackView {
            id: t.id,
            side: self.side,
            domain: t.domain,
            status: t.status,
            model: t.kf.model,
            position: estimate.position(),
            velocity: estimate.velocity(),
            ellipse,
            sigma_up: estimate.p[(2, 2)].max(0.0).sqrt(),
            class_top: t.belief.top_k(p.view_top_k),
            belief: t.belief.clone(),
            best_level: t.belief.best_level,
            quality: p.quality(ellipse.semi_major),
            first_seen: t.created,
            last_update: t.last_update,
            age: self.time - t.last_update,
            updates: t.updates,
            sources: t.sources.iter().copied().collect(),
            sensors: t.sensors.iter().copied().collect(),
            estimate,
        }
    }

    /// One track as a decider may see it.
    pub fn view(&self, id: TrackId, p: &KnowParams) -> Option<TrackView> {
        self.tracks.get(&id).map(|t| self.view_of(t, p))
    }

    /// Every live track, in id order.
    pub fn views(&self, p: &KnowParams) -> Vec<TrackView> {
        self.tracks.values().map(|t| self.view_of(t, p)).collect()
    }

    /// Confirmed tracks only, in id order.
    pub fn confirmed(&self, p: &KnowParams) -> Vec<TrackView> {
        self.tracks.values().filter(|t| t.status == TrackStatus::Confirmed).map(|t| self.view_of(t, p)).collect()
    }

    /// The raw filter covariance of a track (for invariant checks).
    pub fn covariance(&self, id: TrackId) -> Option<&DMatrix<f64>> {
        self.tracks.get(&id).map(|t| &t.kf.p)
    }
}

// Compile-time guard (§17, "truth never reaches a SideView"): `TruthTag` must
// never implement `Serialize`. If it did, both impls below would apply and the
// path `<TruthTag as NotSerialize<_>>::check` would be ambiguous: a compile error.
#[allow(dead_code)]
trait NotSerialize<A> {
    fn check() {}
}
impl<T: ?Sized> NotSerialize<()> for T {}
#[allow(dead_code)]
struct IsSerialize;
impl<T: ?Sized + Serialize> NotSerialize<IsSerialize> for T {}
const _: fn() = || {
    let _ = <TruthTag as NotSerialize<_>>::check;
};
