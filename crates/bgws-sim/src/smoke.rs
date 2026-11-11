//! Smoke (obscurants) in the world: screens of Gaussian puffs
//! ([`bgws_models::signature::SmokeCloud`]) that drift on the surface wind,
//! spread and thin out, and cut what an optical sight sees through them, by
//! band: visible for eyes and day sights, far infrared for thermal sights,
//! near infrared for a laser designator's spot.
//!
//! Where smoke comes from:
//! - **Vehicle smoke dischargers**: an armoured vehicle that sees a missile
//!   launched at it fires a salvo between itself and the launcher, while it
//!   has salvos left (`smoke.vehicle.*`).
//! - **Artillery and mortar smoke rounds**: a screen mission
//!   ([`crate::domains::fires`]), each round a puff where it lands.
//!
//! The sight line is checked only where it crosses a screen: each screen is
//! a circle around its drifting centre (three spreads wide), and the
//! obscurant is integrated along that chord alone.

use crate::sim::Sim;
use bevy_ecs::prelude::Resource;
use bgws_core::{figure, Enu, EntityId, Figure, Figures, Side, SimTime};
use bgws_models::signature::{Band, SignatureParams, SmokeCloud, SmokeKind, SmokePuff};
use serde::Serialize;

pub fn figures() -> Vec<Figure> {
    vec![
        figure!("smoke.vehicle.salvos", 2.0, "salvos", 0.0, 6.0, "default", Default, "smoke discharger salvos an armoured vehicle carries ready to fire"),
        figure!("smoke.vehicle.salvo_g", 2500.0, "g", 200.0, 10000.0, "default", Default, "obscurant in one discharger salvo (several grenades of a few hundred grams)"),
        figure!("smoke.vehicle.distance_m", 30.0, "m", 5.0, 100.0, "default", Default, "how far towards the threat the discharger grenades burst"),
        figure!("smoke.vehicle.warning_p", 0.5, "p", 0.0, 1.0, "default", Default, "chance a crew notices a missile launched at it in time to fire its dischargers, when it has not seen the launcher"),
        figure!("smoke.vehicle.armour_mm", 20.0, "mm", 0.0, 200.0, "default", Default, "frontal KE armour (RHA) from which a land vehicle counts as armoured and carries dischargers"),
        figure!("smoke.screen.cooldown_s", 300.0, "s", 30.0, 1800.0, "default", Default, "a unit asks for a smoke screen on the same enemy at most this often"),
        figure!("smoke.screen.rounds", 6.0, "rounds", 1.0, 24.0, "default", Default, "rounds in a smoke screen mission"),
    ]
}

#[derive(Clone, Copy, Debug)]
pub struct SmokeParams {
    pub vehicle_salvos: u32,
    pub vehicle_salvo_g: f64,
    pub vehicle_distance_m: f64,
    pub vehicle_armour_mm: f64,
    pub vehicle_warning_p: f64,
    pub screen_cooldown_s: f64,
    pub screen_rounds: u32,
}

impl SmokeParams {
    pub fn from_figures(f: &Figures) -> Self {
        let g = |k: &str| f.get(k);
        SmokeParams {
            vehicle_salvos: g("smoke.vehicle.salvos").round() as u32,
            vehicle_salvo_g: g("smoke.vehicle.salvo_g"),
            vehicle_distance_m: g("smoke.vehicle.distance_m"),
            vehicle_armour_mm: g("smoke.vehicle.armour_mm"),
            vehicle_warning_p: g("smoke.vehicle.warning_p"),
            screen_cooldown_s: g("smoke.screen.cooldown_s"),
            screen_rounds: g("smoke.screen.rounds").round() as u32,
        }
    }
}

/// Where a screen came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    Vehicle,
    Artillery,
}

/// One screen: puffs released together, and the wind that carries them.
#[derive(Clone, Debug)]
pub struct Screen {
    pub id: u64,
    pub side: Side,
    pub by: Option<EntityId>,
    pub source: Source,
    pub kind: SmokeKind,
    pub released: SimTime,
    /// Where it was released (east, north; height is above the ground).
    pub at: Enu,
    pub mass_g: f64,
    /// Puffs in a frame where (e, n) is the sim's and the third coordinate
    /// is height above the ground; time is sim seconds.
    pub cloud: SmokeCloud,
}

impl Screen {
    /// Its centre now (drifted) and its reach (three spreads), m.
    pub fn extent(&self, now: SimTime, p: &SignatureParams) -> (Enu, f64) {
        let age = (now - self.released).secs().max(0.0);
        let c = Enu::new(self.at.e + self.cloud.wind[0] * age, self.at.n + self.cloud.wind[1] * age, 0.0);
        let spread = self.cloud.puffs.iter().map(|q| ((q.at[0] - self.at.e).powi(2) + (q.at[1] - self.at.n).powi(2)).sqrt()).fold(0.0, f64::max);
        (c, spread + 3.0 * (p.smoke_sigma0_m + p.smoke_growth_h_mps * age))
    }

    pub fn alive(&self, now: SimTime, p: &SignatureParams) -> bool {
        (now - self.released).secs() < 3.0 * p.smoke_life_s
    }
}

/// The world's smoke.
#[derive(Resource)]
pub struct SmokeState {
    pub screens: Vec<Screen>,
    next: u64,
    pub sig: SignatureParams,
    pub p: SmokeParams,
    /// Discharger salvos each vehicle has fired.
    pub salvos_fired: std::collections::BTreeMap<EntityId, u32>,
    /// When each unit last asked for a screen on each enemy.
    pub asked: std::collections::BTreeMap<(EntityId, EntityId), SimTime>,
}

impl SmokeState {
    pub fn new(f: &Figures) -> Self {
        SmokeState { screens: vec![], next: 1, sig: SignatureParams::from_figures(f), p: SmokeParams::from_figures(f), salvos_fired: Default::default(), asked: Default::default() }
    }

    /// The share of light in `band` that gets through smoke from `a` to `b`
    /// now; each point is (east, north, height above the ground).
    pub fn transmission(&self, band: Band, a: Enu, b: Enu, now: SimTime) -> f64 {
        let mut t = 1.0;
        for s in &self.screens {
            let (c, r) = s.extent(now, &self.sig);
            let Some((u0, u1)) = chord(a, b, c, r) else { continue };
            let lerp = |u: f64| [a.e + (b.e - a.e) * u, a.n + (b.n - a.n) * u, a.u + (b.u - a.u) * u];
            t *= s.cloud.transmission(band, lerp(u0), lerp(u1), now.secs(), &self.sig);
            if t < 1e-6 {
                return 0.0;
            }
        }
        t
    }

    pub fn any(&self) -> bool {
        !self.screens.is_empty()
    }
}

/// Where the segment a→b (horizontally) passes within `r` of `c`: the
/// parameters of entry and exit, or none.
fn chord(a: Enu, b: Enu, c: Enu, r: f64) -> Option<(f64, f64)> {
    let (dx, dy) = (b.e - a.e, b.n - a.n);
    let (fx, fy) = (a.e - c.e, a.n - c.n);
    let aa = dx * dx + dy * dy;
    if aa < 1e-9 {
        return (fx * fx + fy * fy <= r * r).then_some((0.0, 1.0));
    }
    let bb = 2.0 * (fx * dx + fy * dy);
    let cc = fx * fx + fy * fy - r * r;
    let disc = bb * bb - 4.0 * aa * cc;
    if disc < 0.0 {
        return None;
    }
    let sq = disc.sqrt();
    let (u0, u1) = (((-bb - sq) / (2.0 * aa)).max(0.0), ((-bb + sq) / (2.0 * aa)).min(1.0));
    (u0 < u1).then_some((u0, u1))
}

/// Set up the world's smoke (no screens yet).
pub fn setup(s: &mut Sim) {
    let st = SmokeState::new(&s.ctx.figures);
    s.world.insert_resource(st);
}

/// Release a screen of `kind`: `mass_g` split over puffs at `points`
/// (east, north), each `height_m` above the ground. Logged `smoke.screen`.
#[allow(clippy::too_many_arguments)]
pub fn release(s: &mut Sim, side: Side, by: Option<EntityId>, source: Source, kind: SmokeKind, points: &[Enu], height_m: f64, mass_g: f64) -> u64 {
    let now = s.ctx.now;
    if points.is_empty() {
        return 0;
    }
    let n = points.len() as f64;
    let at = Enu::new(points.iter().map(|p| p.e).sum::<f64>() / n, points.iter().map(|p| p.n).sum::<f64>() / n, 0.0);
    let w = s.ctx.weather.sample(at.e, at.n, now).wind_surface;
    let puffs = points.iter().map(|p| SmokePuff { kind, mass_g: mass_g / n, at: [p.e, p.n, height_m], released_s: now.secs() }).collect();
    let Some(mut st) = s.world.get_resource_mut::<SmokeState>() else { return 0 };
    let id = st.next;
    st.next += 1;
    st.screens.push(Screen { id, side, by, source, kind, released: now, at, mass_g, cloud: SmokeCloud { puffs, wind: [w.0, w.1] } });
    let text = match (source, by) {
        (Source::Vehicle, Some(b)) => format!("{} fires its smoke dischargers", crate::systems::name(s, b)),
        (Source::Artillery, _) => "smoke rounds land: a screen".to_string(),
        _ => "smoke".to_string(),
    };
    let mut ev = s.ctx.log.push(now, "smoke.screen").side(side).text(text).data(serde_json::json!({ "screen": id, "source": source, "kind": format!("{kind:?}").to_lowercase(), "e": at.e, "n": at.n, "massG": mass_g, "puffs": n }));
    if let Some(b) = by {
        ev = ev.entity(b);
    }
    let _ = ev;
    id
}

/// Does this entity carry smoke dischargers (an armoured land vehicle)?
pub fn has_dischargers(s: &Sim, id: EntityId) -> bool {
    let Some(&e) = s.ctx.order.get(id.index()) else { return false };
    let Some(i) = s.world.get::<crate::components::Ident>(e) else { return false };
    let min = s.world.get_resource::<SmokeState>().map(|st| st.p.vehicle_armour_mm).unwrap_or(20.0);
    i.domain == bgws_catalogue::Domain::Land && i.platform.body.armour.ke.front >= min
}

/// A missile has been launched at `target` by `firer` from `from`: an
/// armoured vehicle with salvos left puts a screen between itself and the
/// launcher, if its crew notices in time (at once if it has the launcher in
/// sight or in mind, else with `smoke.vehicle.warning_p`).
pub fn missile_warning(s: &mut Sim, target: EntityId, firer: EntityId, from: Enu) {
    if !has_dischargers(s, target) || crate::remote::is_remote(s, s.ctx.entity(target)) {
        return;
    }
    let e = s.ctx.entity(target);
    let (Some(k), Some(h), Some(i)) = (s.world.get::<crate::components::Kin>(e).copied(), s.world.get::<crate::components::Health>(e), s.world.get::<crate::components::Ident>(e)) else { return };
    if h.destroyed {
        return;
    }
    let side = i.side;
    let Some(st) = s.world.get_resource::<SmokeState>() else { return };
    let (p, fired) = (st.p, st.salvos_fired.get(&target).copied().unwrap_or(0));
    if fired >= p.vehicle_salvos {
        return;
    }
    let knows = s.world.get::<crate::components::Aware>(e).is_some_and(|a| a.seen.get(&firer).is_some_and(|x| (s.ctx.now - x.last).secs() < 60.0));
    if !knows {
        use rand::Rng;
        let mut rng = s.ctx.rng("smoke.warning", target.0 as u64, s.ctx.now.0 as u64);
        if rng.gen::<f64>() >= p.vehicle_warning_p {
            return;
        }
    }
    let d = from.sub(k.pos);
    let len = d.horizontal().max(1.0);
    let (ux, uy) = (d.e / len, d.n / len);
    // A fan of grenades across the threat's line, `distance_m` out.
    let c = Enu::new(k.pos.e + ux * p.vehicle_distance_m, k.pos.n + uy * p.vehicle_distance_m, 0.0);
    let points: Vec<Enu> = [-12.0, -4.0, 4.0, 12.0].iter().map(|o| Enu::new(c.e - uy * o, c.n + ux * o, 0.0)).collect();
    if let Some(mut st) = s.world.get_resource_mut::<SmokeState>() {
        *st.salvos_fired.entry(target).or_insert(0) += 1;
    }
    // Red phosphorus or similar: visual and some infrared.
    release(s, side, Some(target), Source::Vehicle, SmokeKind::Wp, &points, 2.0, p.vehicle_salvo_g);
}

/// 1 Hz: screens that have thinned away are gone.
pub fn base(s: &mut Sim) {
    let now = s.ctx.now;
    if let Some(mut st) = s.world.get_resource_mut::<SmokeState>() {
        let sig = st.sig.clone();
        st.screens.retain(|x| x.alive(now, &sig));
    }
}

/// The band a sensor sees in, for smoke.
pub fn band_of(sensor: bgws_catalogue::SensorKind) -> Band {
    match sensor {
        bgws_catalogue::SensorKind::Thermal => Band::FarIr,
        _ => Band::Visual,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chords_are_where_the_line_crosses_the_circle() {
        let (a, b) = (Enu::new(-100.0, 0.0, 0.0), Enu::new(100.0, 0.0, 0.0));
        let (u0, u1) = chord(a, b, Enu::new(0.0, 0.0, 0.0), 50.0).unwrap();
        assert!((u0 - 0.25).abs() < 1e-9 && (u1 - 0.75).abs() < 1e-9);
        assert!(chord(a, b, Enu::new(0.0, 60.0, 0.0), 50.0).is_none());
        assert!(chord(a, Enu::new(-80.0, 0.0, 0.0), Enu::new(0.0, 0.0, 0.0), 50.0).is_none(), "ends before the circle");
    }

    #[test]
    fn a_screen_blocks_sight_through_it_by_band_and_thins_away() {
        let f = crate::default_figures();
        let mut st = SmokeState::new(&f);
        let t0 = SimTime::from_secs(100.0);
        let puffs = [-12.0, -4.0, 4.0, 12.0].iter().map(|&y| SmokePuff { kind: SmokeKind::Wp, mass_g: 625.0, at: [0.0, y, 2.0], released_s: 100.0 }).collect();
        st.screens.push(Screen { id: 1, side: Side::Blue, by: None, source: Source::Vehicle, kind: SmokeKind::Wp, released: t0, at: Enu::new(0.0, 0.0, 0.0), mass_g: 2500.0, cloud: SmokeCloud { puffs, wind: [0.0, 0.0] } });
        let (a, b) = (Enu::new(-1000.0, 0.0, 2.0), Enu::new(30.0, 0.0, 1.5));
        let t = |band, secs| st.transmission(band, a, b, SimTime::from_secs(secs));
        let (vis, ir) = (t(Band::Visual, 110.0), t(Band::FarIr, 110.0));
        assert!(vis < 0.05, "visible through fresh smoke: {vis}");
        assert!(ir > vis, "thermal sees better through phosphorus smoke: {ir} vs {vis}");
        // A line well clear of it is untouched.
        assert_eq!(st.transmission(Band::Visual, Enu::new(-1000.0, 500.0, 2.0), Enu::new(30.0, 500.0, 1.5), SimTime::from_secs(110.0)), 1.0);
        // Ten minutes on it has spread and thinned.
        assert!(t(Band::Visual, 700.0) > vis, "{} vs {vis}", t(Band::Visual, 700.0));
    }
}
