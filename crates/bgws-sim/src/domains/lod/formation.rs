//! Formations, aggregation and aggregated combat (§6.4, §8.13; R8, P14).
//!
//! Membership is fixed at setup: land ground movers grouped by their
//! scenario `parent`, the rest by side and proximity (single linkage within
//! `sim.lod.cluster_m`). Rear logistics are kept out of formations and run at
//! the Far level when far from every enemy.
//!
//! Aggregation folds the fit vehicles of every member into one
//! `bgws_ops::aggregate::Formation` (one record per vehicle, so losses are
//! counted in vehicles, as the entity model counts them). Disaggregation draws
//! which vehicles died from the formation's keyed stream (formation, epoch):
//! the draw is a fixed shuffle cut at the loss count, so the killed set only
//! grows as losses grow. The sim applies it to `Health` after every aggregate
//! attrition step (strength, breakpoints and the AAR stay true while
//! aggregated) and once more on disaggregation; `Health` is never raised, so
//! killed stays killed.

use super::{LodParams, LodState};
use crate::components::*;
use crate::sim::Sim;
use crate::systems::{alive, arrive, hypothesis_of, mover_class};
use bevy_ecs::prelude::Entity;
use bgws_catalogue::{Domain, Platform};
use bgws_core::{Enu, EntityId, Figures, Side, SimTime};
use bgws_env::LandCover;
use bgws_know::Hypothesis;
use bgws_models::people::MoraleState;
use bgws_ops::aggregate::{self as agg, AttritionCoeffs, AttritionStep, EngagementEnv, FireType, Formation, MemberRecord, Posture as AggPosture, TerrainClass};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Attrition coefficients by formation-type pair (§8.13). The defaults are
/// the **[default]** unfitted rate; a table fitted from entity runs
/// (`bgws_ops::aggregate::fit_attrition`) is loaded with
/// `domains::lod::load_attrition_table`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttritionTable {
    /// Where the coefficients came from ("unfitted", or the fit's provenance).
    pub source: String,
    /// For any pair without its own entry.
    pub default: AttritionCoeffs,
    /// Keyed `"a|b"` (formation kinds): `k_a_on_b` is a's kills on b per
    /// effective shooter-hour. The reverse key is used swapped.
    #[serde(default)]
    pub pairs: BTreeMap<String, AttritionCoeffs>,
}

impl AttritionTable {
    pub fn unfitted(f: &Figures) -> Self {
        AttritionTable { source: "unfitted".into(), default: AttritionCoeffs::unfitted(f), pairs: BTreeMap::new() }
    }

    /// The coefficients for `a` against `b`.
    pub fn coeffs(&self, a: &str, b: &str) -> AttritionCoeffs {
        if let Some(c) = self.pairs.get(&format!("{a}|{b}")) {
            *c
        } else if let Some(c) = self.pairs.get(&format!("{b}|{a}")) {
            AttritionCoeffs { k_a_on_b: c.k_b_on_a, k_b_on_a: c.k_a_on_b }
        } else {
            self.default
        }
    }
}

/// A formation and, while aggregated, its aggregate state.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormationState {
    pub id: u64,
    pub side: Side,
    pub name: String,
    /// armour, mech, infantry, fires or support (the attrition table's key).
    pub kind: String,
    pub members: Vec<EntityId>,
    pub agg: Option<Aggregate>,
}

impl FormationState {
    pub fn aggregated(&self) -> bool {
        self.agg.is_some()
    }
}

/// The aggregate state of a formation.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Aggregate {
    pub formation: Formation,
    /// Record index → (member, vehicle slot).
    pub vehicles: Vec<(EntityId, u32)>,
    pub since: SimTime,
    /// The formation mover's pace: its slowest moving member's.
    pub speed_ms: f64,
    pub combat_h: f64,
}

/// The kind of a platform for the attrition table.
pub fn kind_of(p: &Platform) -> &'static str {
    match hypothesis_of(p) {
        Hypothesis::Mbt => "armour",
        Hypothesis::Ifv | Hypothesis::Apc => "mech",
        Hypothesis::Infantry => "infantry",
        Hypothesis::Artillery => "fires",
        _ => "support",
    }
}

/// Rear logistics (convoys, ambulances): never in a formation; Far when far.
pub fn logistic(p: &Platform) -> bool {
    let t = format!("{} {}", p.id, p.class).to_lowercase();
    ["convoy", "medevac", "ambulance", "logist", "supply", "tanker"].iter().any(|k| t.contains(k))
}

/// Group the land units into formations (setup).
pub(crate) fn build(s: &mut Sim, st: &mut LodState) {
    let mut by_parent: BTreeMap<String, Vec<EntityId>> = BTreeMap::new();
    let mut loose: Vec<(EntityId, Side, Enu)> = Vec::new();
    for &e in &s.ctx.order {
        let (Some(i), Some(h), Some(k)) = (s.world.get::<Ident>(e), s.world.get::<Health>(e), s.world.get::<Kin>(e)) else { continue };
        if let Some(a) = s.world.get::<Arms>(e) {
            let start = h.start.max(1) as f64;
            st.basic.insert(i.id, a.mounts.iter().map(|m| m.rounds as f64 / start).collect());
        }
        if i.domain != Domain::Land || i.dummy || i.side == Side::Neutral {
            continue;
        }
        if logistic(&i.platform) {
            st.logistics.push(i.id);
            continue;
        }
        if !mover_class(i.platform.mover.kind).1 {
            continue;
        }
        match s.ctx.scenario.forces.get(i.id.index()).and_then(|f| f.parent.clone()) {
            Some(p) => by_parent.entry(p).or_default().push(i.id),
            None => loose.push((i.id, i.side, k.pos)),
        }
    }
    let mut groups: Vec<(Side, String, Vec<EntityId>)> = Vec::new();
    for (parent, members) in by_parent {
        let side = s.world.get::<Ident>(s.ctx.entity(members[0])).map(|i| i.side).unwrap_or(Side::Neutral);
        let label = s.ctx.by_name.get(&parent).and_then(|&id| s.world.get::<Ident>(s.ctx.entity(id))).map(|i| i.label.clone()).unwrap_or(parent);
        groups.push((side, label, members));
    }
    // Single linkage by side within cluster_m (union-find in id order).
    let n = loose.len();
    let mut root: Vec<usize> = (0..n).collect();
    fn find(r: &mut [usize], mut x: usize) -> usize {
        while r[x] != x {
            r[x] = r[r[x]];
            x = r[x];
        }
        x
    }
    for i in 0..n {
        for j in i + 1..n {
            if loose[i].1 == loose[j].1 && loose[i].2.sub(loose[j].2).horizontal() <= st.p.cluster_m {
                let (a, b) = (find(&mut root, i), find(&mut root, j));
                if a != b {
                    root[a.max(b)] = a.min(b);
                }
            }
        }
    }
    let mut clusters: BTreeMap<usize, Vec<EntityId>> = BTreeMap::new();
    for (i, x) in loose.iter().enumerate() {
        let r = find(&mut root, i);
        clusters.entry(r).or_default().push(x.0);
    }
    for (r, members) in clusters {
        let label = s.world.get::<Ident>(s.ctx.entity(members[0])).map(|i| format!("{} group", i.label)).unwrap_or_default();
        groups.push((loose[r].1, label, members));
    }
    groups.sort_by_key(|g| g.2[0]);
    for (side, name, members) in groups {
        let mut tally: BTreeMap<&'static str, u32> = BTreeMap::new();
        for &m in &members {
            let e = s.ctx.entity(m);
            if let (Some(i), Some(h)) = (s.world.get::<Ident>(e), s.world.get::<Health>(e)) {
                *tally.entry(kind_of(&i.platform)).or_default() += h.start;
            }
        }
        let kind = tally.iter().max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0))).map(|(k, _)| k.to_string()).unwrap_or_else(|| "support".into());
        let id = st.formations.len() as u64;
        for &m in &members {
            st.of.insert(m, id as usize);
        }
        st.formations.push(FormationState { id, side, name, kind, members, agg: None });
    }
}

/// Centre and radius of the living members.
fn centre(s: &Sim, members: &[EntityId]) -> Option<(Enu, f64)> {
    let pts: Vec<Enu> = members.iter().map(|&m| s.ctx.entity(m)).filter(|&e| alive(s, e)).filter_map(|e| s.world.get::<Kin>(e).map(|k| k.pos)).collect();
    if pts.is_empty() {
        return None;
    }
    let n = pts.len() as f64;
    let c = Enu::new(pts.iter().map(|p| p.e).sum::<f64>() / n, pts.iter().map(|p| p.n).sum::<f64>() / n, 0.0);
    let r = pts.iter().map(|p| p.sub(c).horizontal()).fold(0.0, f64::max);
    Some((c, r))
}

/// Not in an engagement: nothing seen or suspected, no recent fire either way,
/// no decision out, steady.
fn calm(s: &Sim, e: Entity, p: &LodParams) -> bool {
    if !alive(s, e) {
        return true;
    }
    let now = s.ctx.now;
    let (Some(aw), Some(a), Some(c)) = (s.world.get::<Aware>(e), s.world.get::<Act>(e), s.world.get::<Crew>(e)) else { return false };
    aw.seen.is_empty()
        && aw.suspects.is_empty()
        && a.last_shot.map(|t| (now - t).secs() > p.calm_s).unwrap_or(true)
        && c.last_incoming.is_none()
        && !a.deciding
        && c.morale.state == MoraleState::Steady
        && !matches!(a.activity, Activity::Engage { .. } | Activity::Wait { .. })
}

fn set_lod(s: &mut Sim, id: EntityId, lod: Lod) {
    let e = s.ctx.entity(id);
    if let Some(mut l) = s.world.get_mut::<Lod>(e) {
        *l = lod;
    }
}

/// Stores as a fraction of the basic load of its fit vehicles.
fn stores_frac(s: &Sim, st: &LodState, id: EntityId) -> f64 {
    let e = s.ctx.entity(id);
    let basic: f64 = st.basic.get(&id).map(|v| v.iter().sum()).unwrap_or(0.0);
    let (Some(a), Some(h)) = (s.world.get::<Arms>(e), s.world.get::<Health>(e)) else { return 1.0 };
    if basic <= 0.0 || h.fit == 0 {
        return 1.0;
    }
    (a.rounds() as f64 / (basic * h.fit as f64)).clamp(0.0, 1.0)
}

/// The formation's posture from what its members are doing.
fn posture_of(s: &Sim, members: &[EntityId]) -> AggPosture {
    let mut tally = [0u32; 4];
    for &m in members {
        let e = s.ctx.entity(m);
        if !alive(s, e) {
            continue;
        }
        let (Some(a), Some(h)) = (s.world.get::<Act>(e), s.world.get::<Health>(e)) else { continue };
        let i = match a.activity {
            Activity::Move { .. } => 0,
            Activity::Withdraw { .. } => 3,
            _ if a.posture == Posture::HullDown => 2,
            _ => 1,
        };
        tally[i] += h.fit;
    }
    let best = (0..4).max_by(|&a, &b| tally[a].cmp(&tally[b]).then(b.cmp(&a))).unwrap_or(1);
    [AggPosture::Attack, AggPosture::HastyDefence, AggPosture::PreparedDefence, AggPosture::Delay][best]
}

/// The pace of the formation mover: the slowest moving member's speed over
/// its own ground, as the entity mover would set it (without suppression).
fn pace(s: &Sim, members: &[EntityId]) -> f64 {
    let mut v = f64::INFINITY;
    for &m in members {
        let e = s.ctx.entity(m);
        if !alive(s, e) {
            continue;
        }
        let (Some(i), Some(a), Some(k), Some(h)) = (s.world.get::<Ident>(e), s.world.get::<Act>(e), s.world.get::<Kin>(e), s.world.get::<Health>(e)) else { continue };
        let (mode, withdrawing) = match &a.activity {
            Activity::Move { mode, .. } => (*mode, false),
            Activity::Withdraw { .. } => (MoveModeS::March, true),
            _ => continue,
        };
        if !h.can_move() {
            continue;
        }
        let pm = &i.platform.mover;
        let (class, _) = mover_class(pm.kind);
        let going = s.ctx.ground.going(k.pos.e, k.pos.n, class);
        let off_road = pm.cross_country_kmh.max(1.0).min(going.speed_kmh.max(0.0));
        let cap = pm.road_kmh.max(pm.cross_country_kmh) * bgws_models::ground::mode_cap(mode.into(), &s.ctx.p.ground);
        let kmh = if withdrawing { off_road } else { off_road.min(cap) };
        v = v.min(kmh / 3.6);
    }
    if v.is_finite() {
        v
    } else {
        0.0
    }
}

/// Fold a formation (§6.4).
fn aggregate_formation(s: &mut Sim, st: &mut LodState, fi: usize, nearest_m: f64) {
    let now = s.ctx.now;
    let (id, side, kind, members, name) = {
        let f = &st.formations[fi];
        (f.id, f.side, f.kind.clone(), f.members.clone(), f.name.clone())
    };
    let mut vehicles = Vec::new();
    let mut records = Vec::new();
    for &m in &members {
        let e = s.ctx.entity(m);
        if !alive(s, e) {
            continue;
        }
        let (Some(h), Some(c), Some(i)) = (s.world.get::<Health>(e), s.world.get::<Crew>(e), s.world.get::<Ident>(e)) else { continue };
        let stores = stores_frac(s, st, m);
        let class = kind_of(&i.platform).to_string();
        for slot in 0..h.fit {
            records.push(MemberRecord { entity: EntityId(vehicles.len() as u32), class: class.clone(), alive: true, stores, fatigue: c.fatigue.physical.clamp(0.0, 1.0) });
            vehicles.push((m, slot));
        }
    }
    if vehicles.is_empty() {
        return;
    }
    st.epoch += 1;
    let fire = if kind == "fires" { FireType::Area } else { FireType::Aimed };
    let formation = agg::aggregate(id, side, &records, st.epoch, fire, posture_of(s, &members));
    for &m in &members {
        set_lod(s, m, Lod::Aggregated);
    }
    let speed_ms = pace(s, &members);
    let n = vehicles.len();
    let units = members.iter().filter(|&&m| alive(s, s.ctx.entity(m))).count();
    s.ctx
        .log
        .push(now, "lod.aggregate")
        .side(side)
        .entity(members[0])
        .text(format!("{name} aggregated: {n} vehicles in {units} units, nearest enemy {:.1} km", nearest_m / 1000.0))
        .data(serde_json::json!({ "formation": id, "kind": kind, "vehicles": n, "members": members.iter().map(|m| m.0).collect::<Vec<_>>(), "epoch": st.epoch, "nearestM": nearest_m.round(), "stores": formation.stores.mean, "fatigue": formation.fatigue.mean }));
    st.formations[fi].agg = Some(Aggregate { formation, vehicles, since: now, speed_ms, combat_h: 0.0 });
}

/// Survivors per member from the keyed draw, with their sampled stores and fatigue.
type Draw = BTreeMap<EntityId, (u32, Vec<f64>, Vec<f64>)>;

fn draw(a: &Aggregate, seed: bgws_core::Seed) -> Draw {
    let mut per: Draw = BTreeMap::new();
    for (m, _) in &a.vehicles {
        per.entry(*m).or_default();
    }
    for r in a.formation.disaggregate(seed) {
        let Some(&(m, _)) = a.vehicles.get(r.entity.index()) else { continue };
        if r.alive {
            let x = per.entry(m).or_default();
            x.0 += 1;
            x.1.push(r.stores);
            x.2.push(r.fatigue);
        }
    }
    per
}

/// Apply the drawn survivors to `Health` (never raising it). Returns vehicles killed.
fn apply_draw(s: &mut Sim, side: Side, d: &Draw) -> u32 {
    let now = s.ctx.now;
    let mut killed = 0;
    for (&m, (alive_n, _, _)) in d {
        let e = s.ctx.entity(m);
        let destroyed = {
            let Some(mut h) = s.world.get_mut::<Health>(e) else { continue };
            if *alive_n >= h.fit || h.destroyed {
                continue;
            }
            let k = h.fit - alive_n;
            h.fit = *alive_n;
            for _ in 0..k {
                h.losses.push(now);
            }
            killed += k;
            let frac = k as f64 / h.start as f64;
            let gone = h.fit == 0;
            if gone {
                h.destroyed = true;
            }
            if let Some(mut c) = s.world.get_mut::<Crew>(e) {
                c.new_casualties += frac;
            }
            gone
        };
        if destroyed {
            let text = format!("{} is destroyed (aggregate combat)", crate::systems::name(s, m));
            s.ctx.log.push(now, "unit.destroyed").side(side).entity(m).text(text);
        }
    }
    killed
}

/// Recreate the entities (§6.4).
pub(crate) fn disaggregate_formation(s: &mut Sim, st: &mut LodState, fi: usize, why: &str) {
    let now = s.ctx.now;
    let Some(a) = st.formations[fi].agg.take() else { return };
    let (id, side, name, members) = {
        let f = &st.formations[fi];
        (f.id, f.side, f.name.clone(), f.members.clone())
    };
    let d = draw(&a, s.ctx.seed);
    let killed = apply_draw(s, side, &d);
    for (&m, (n, stores, fatigue)) in &d {
        if *n == 0 {
            continue;
        }
        let e = s.ctx.entity(m);
        let mean = |v: &Vec<f64>| v.iter().sum::<f64>() / v.len() as f64;
        // Stores: down to the sampled share of basic load, never up.
        let basic: f64 = st.basic.get(&m).map(|v| v.iter().sum()).unwrap_or(0.0);
        let fit = s.world.get::<Health>(e).map(|h| h.fit).unwrap_or(0);
        if let Some(mut arms) = s.world.get_mut::<Arms>(e) {
            let now_total = arms.rounds() as f64;
            let target = basic * fit as f64 * mean(stores);
            if basic > 0.0 && now_total > 0.0 && target < now_total {
                let f = target / now_total;
                for mt in arms.mounts.iter_mut() {
                    mt.rounds = (mt.rounds as f64 * f).floor() as u32;
                }
            }
        }
        if let Some(mut c) = s.world.get_mut::<Crew>(e) {
            c.fatigue.physical = c.fatigue.physical.max(mean(fatigue)).clamp(0.0, 1.0);
        }
    }
    for &m in &members {
        set_lod(s, m, Lod::Entity);
    }
    let survivors: u32 = d.values().map(|x| x.0).sum();
    s.ctx
        .log
        .push(now, "lod.disaggregate")
        .side(side)
        .entity(members[0])
        .text(format!("{name} back to entity level ({why}): {survivors} of {} vehicles, {} lost while aggregated", a.formation.initial_strength, a.formation.losses()))
        .data(serde_json::json!({ "formation": id, "epoch": a.formation.epoch, "initial": a.formation.initial_strength, "survivors": survivors, "losses": a.formation.losses(), "killedNow": killed, "aggregatedS": (now - a.since).secs(), "combatH": a.combat_h, "stores": a.formation.stores.mean, "fatigue": a.formation.fatigue.mean, "why": why }));
}

/// The formation mover (each base tick): every moving member follows its own
/// route at the formation's pace.
pub(crate) fn move_aggregated(s: &mut Sim, st: &mut LodState, dt: f64) {
    let now = s.ctx.now;
    let arrive_m = s.ctx.p.arrive_m;
    for f in &st.formations {
        let Some(a) = &f.agg else { continue };
        let v = a.speed_ms;
        for &m in &f.members {
            let e = s.ctx.entity(m);
            if !alive(s, e) || s.world.get::<Lod>(e).copied() != Some(Lod::Aggregated) {
                continue;
            }
            let can_move = s.world.get::<Health>(e).map(|h| h.can_move()).unwrap_or(false);
            let route = match &s.world.get::<Act>(e).expect("act").activity {
                Activity::Move { route, .. } | Activity::Withdraw { route, .. } => route.first().copied(),
                _ => {
                    if let Some(mut k) = s.world.get_mut::<Kin>(e) {
                        k.vel = Enu::default();
                    }
                    continue;
                }
            };
            if !can_move {
                continue;
            }
            let Some(wp) = route else {
                arrive(s, e);
                continue;
            };
            let k = *s.world.get::<Kin>(e).expect("kin");
            let d = Enu::new(wp.e - k.pos.e, wp.n - k.pos.n, 0.0);
            let dist = d.horizontal();
            let step = (v * dt).min(dist);
            let (ne, nn) = if dist > 1e-6 { (k.pos.e + d.e / dist * step, k.pos.n + d.n / dist * step) } else { (wp.e, wp.n) };
            let nu = s.ctx.ground.height(ne, nn);
            {
                let mut km = s.world.get_mut::<Kin>(e).expect("kin");
                km.pos = Enu::new(ne, nn, nu);
                km.vel = if dt > 0.0 { Enu::new((ne - k.pos.e) / dt, (nn - k.pos.n) / dt, 0.0) } else { Enu::default() };
                if dist > 1e-6 {
                    km.heading = d.bearing();
                }
            }
            if let Some(mut sig) = s.world.get_mut::<SigState>(e) {
                sig.last_moved = Some(now);
            }
            if dist - step <= arrive_m {
                let mut a = s.world.get_mut::<Act>(e).expect("act");
                if let Activity::Move { route, .. } | Activity::Withdraw { route, .. } = &mut a.activity {
                    route.remove(0);
                }
            }
        }
    }
}

fn terrain_class(c: LandCover) -> TerrainClass {
    match c {
        LandCover::Open | LandCover::Crops | LandCover::Bare | LandCover::Water => TerrainClass::Open,
        LandCover::Scrub | LandCover::ForestLight | LandCover::Wetland => TerrainClass::Mixed,
        LandCover::ForestDense => TerrainClass::Close,
        LandCover::UrbanLight | LandCover::UrbanDense => TerrainClass::Urban,
    }
}

/// The 0.1 Hz pass: aggregate, disaggregate, fight, and the Far level.
pub(crate) fn pass(s: &mut Sim, st: &mut LodState, dt: f64) {
    let p = st.p.clone();
    // Entity-level enemies by side (truth: aggregated members are invisible
    // to entity sensing, so an entity-level enemy must never reach them
    // unseen), every enemy for the Far test, and each side's tracks.
    let mut entity_pts: BTreeMap<Side, Vec<Enu>> = BTreeMap::new();
    let mut any_pts: BTreeMap<Side, Vec<Enu>> = BTreeMap::new();
    for &e in &s.ctx.order {
        let (Some(i), Some(k), Some(l)) = (s.world.get::<Ident>(e), s.world.get::<Kin>(e), s.world.get::<Lod>(e)) else { continue };
        if !alive(s, e) || i.dummy || !matches!(i.domain, Domain::Land | Domain::Air | Domain::Surface | Domain::Subsurface) {
            continue;
        }
        any_pts.entry(i.side).or_default().push(k.pos);
        if *l == Lod::Entity {
            entity_pts.entry(i.side).or_default().push(k.pos);
        }
    }
    let track_pts: BTreeMap<Side, Vec<Enu>> = s.ctx.tracks.iter().map(|(side, t)| (*side, t.confirmed(&s.ctx.p.know).into_iter().map(|v| v.position).collect())).collect();
    let nearest = |pts: Option<&Vec<Enu>>, c: Enu| pts.map(|v| v.iter().map(|q| q.sub(c).horizontal()).fold(f64::INFINITY, f64::min)).unwrap_or(f64::INFINITY);
    for fi in 0..st.formations.len() {
        let (side, members, aggregated) = {
            let f = &st.formations[fi];
            (f.side, f.members.clone(), f.aggregated())
        };
        let Some((c, r)) = centre(s, &members) else {
            if aggregated {
                disaggregate_formation(s, st, fi, "no survivors");
            }
            continue;
        };
        let d_ent = nearest(entity_pts.get(&side.enemy()), c) - r;
        let d_trk = nearest(track_pts.get(&side), c) - r;
        let d = d_ent.min(d_trk);
        if aggregated {
            if d < p.aggregate_m * p.hysteresis {
                let why = if d_trk <= d_ent { format!("an enemy track {:.1} km away", d_trk.max(0.0) / 1000.0) } else { format!("an enemy unit {:.1} km away", d_ent.max(0.0) / 1000.0) };
                disaggregate_formation(s, st, fi, &why);
            } else {
                let pace = pace(s, &members);
                let posture = posture_of(s, &members);
                if let Some(a) = st.formations[fi].agg.as_mut() {
                    a.speed_ms = pace;
                    a.formation.posture = posture;
                }
            }
        } else if d > p.aggregate_m && members.iter().all(|&m| calm(s, s.ctx.entity(m), &p)) {
            aggregate_formation(s, st, fi, d);
        }
    }
    attrition(s, st, dt);
    far(s, st, &any_pts);
}

/// Aggregate attrition between aggregated enemy formations in contact (§8.13).
fn attrition(s: &mut Sim, st: &mut LodState, dt: f64) {
    let now = s.ctx.now;
    let dt_h = dt / 3600.0;
    let figures = s.ctx.figures.clone();
    let live: Vec<(usize, Enu, f64)> = (0..st.formations.len())
        .filter(|&i| st.formations[i].agg.as_ref().map(|a| a.formation.strength > 0).unwrap_or(false))
        .filter_map(|i| centre(s, &st.formations[i].members).map(|(c, r)| (i, c, r)))
        .collect();
    for x in 0..live.len() {
        for y in x + 1..live.len() {
            let (i, ci, ri) = live[x];
            let (j, cj, rj) = live[y];
            let (si, sj) = (st.formations[i].side, st.formations[j].side);
            if si.enemy() != sj || si == Side::Neutral {
                continue;
            }
            let gap = ci.sub(cj).horizontal() - ri - rj;
            if gap > st.p.contact_m {
                continue;
            }
            let (Some(mut a), Some(mut b)) = (st.formations[i].agg.take(), st.formations[j].agg.take()) else { continue };
            if a.formation.strength == 0 || b.formation.strength == 0 {
                st.formations[i].agg = Some(a);
                st.formations[j].agg = Some(b);
                continue;
            }
            let mid = Enu::new((ci.e + cj.e) / 2.0, (ci.n + cj.n) / 2.0, 0.0);
            let env = EngagementEnv { terrain: terrain_class(s.ctx.ground.land_cover(mid.e, mid.n)), supply: (a.formation.stores.mean, b.formation.stores.mean) };
            let k = st.table.coeffs(&st.formations[i].kind, &st.formations[j].kind);
            let mut rng = s.ctx.rng("lod.attrition", st.formations[i].id, st.formations[j].id);
            let step: AttritionStep = agg::attrition_step(&mut a.formation, &mut b.formation, &k, &env, dt_h, &figures, &mut rng);
            a.formation.consume(st.p.consume_per_combat_h * dt_h);
            b.formation.consume(st.p.consume_per_combat_h * dt_h);
            let first = a.combat_h == 0.0 || b.combat_h == 0.0;
            a.combat_h += dt_h;
            b.combat_h += dt_h;
            if first || step.losses_a + step.losses_b > 0 {
                let text = format!(
                    "{} and {} fight in aggregate ({:.1} km apart): {} and {} vehicles lost, {} and {} left",
                    st.formations[i].name,
                    st.formations[j].name,
                    gap.max(0.0) / 1000.0,
                    step.losses_a,
                    step.losses_b,
                    a.formation.strength,
                    b.formation.strength
                );
                s.ctx.log.push(now, "lod.attrition").side(si).entity(st.formations[i].members[0]).target(st.formations[j].members[0]).text(text).data(serde_json::json!({
                    "a": st.formations[i].id, "b": st.formations[j].id, "step": step, "coeffs": k, "table": st.table.source, "terrain": env.terrain, "postures": [a.formation.posture, b.formation.posture],
                }));
            }
            if step.losses_a > 0 {
                apply_draw(s, si, &draw(&a, s.ctx.seed));
            }
            if step.losses_b > 0 {
                apply_draw(s, sj, &draw(&b, s.ctx.seed));
            }
            st.formations[i].agg = Some(a);
            st.formations[j].agg = Some(b);
        }
    }
}

/// Rear logistics far from every enemy run at the Far level (no tactical sensing).
fn far(s: &mut Sim, st: &LodState, any_pts: &BTreeMap<Side, Vec<Enu>>) {
    let now = s.ctx.now;
    let out = st.p.aggregate_m * st.p.far_factor;
    for &id in &st.logistics {
        let e = s.ctx.entity(id);
        if !alive(s, e) {
            continue;
        }
        let (Some(i), Some(k), Some(l)) = (s.world.get::<Ident>(e), s.world.get::<Kin>(e), s.world.get::<Lod>(e)) else { continue };
        let side = i.side;
        let d = any_pts.get(&side.enemy()).map(|v| v.iter().map(|q| q.sub(k.pos).horizontal()).fold(f64::INFINITY, f64::min)).unwrap_or(f64::INFINITY);
        let next = match *l {
            Lod::Entity if d > out => Lod::Far,
            Lod::Far if d < out * st.p.hysteresis => Lod::Entity,
            other => other,
        };
        if next != *l {
            set_lod(s, id, next);
            let text = format!("{} {} (nearest enemy {:.1} km)", crate::systems::name(s, id), if next == Lod::Far { "runs at the Far level" } else { "back to entity level" }, d / 1000.0);
            s.ctx.log.push(now, if next == Lod::Far { "lod.far" } else { "lod.near" }).side(side).entity(id).text(text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_lookup_swaps_reverse_pairs() {
        let f = crate::default_figures();
        let mut t = AttritionTable::unfitted(&f);
        t.pairs.insert("armour|infantry".into(), AttritionCoeffs { k_a_on_b: 0.5, k_b_on_a: 0.1 });
        assert_eq!(t.coeffs("armour", "infantry").k_a_on_b, 0.5);
        let r = t.coeffs("infantry", "armour");
        assert_eq!((r.k_a_on_b, r.k_b_on_a), (0.1, 0.5));
        assert_eq!(t.coeffs("mech", "mech"), t.default);
        let json = serde_json::to_string(&t).unwrap();
        assert_eq!(serde_json::from_str::<AttritionTable>(&json).unwrap(), t);
    }
}
