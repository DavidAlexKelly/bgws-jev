//! Laser designation: a semi-active laser munition (seeker kind `laser`,
//! the Hellfire-class AGM) homes on a spot its designator keeps on the
//! target until it lands. Here the launcher designates (an attack
//! helicopter or a drone lasing its own shot).
//!
//! Each second the spot follows the target, and the designation holds while
//! the designator is alive, has the target in sight (on the ground: it
//! sighted it in the last 5 s, its tracker holding between sightings; in the
//! air: a clear line over the ground), and the laser's near infrared gets
//! through any smoke between them (there and back). At impact a munition that has its spot homes on the target where
//! it is; one that has lost it flies on unguided and lands
//! `laser.lost_cep_m` about its aim. A target's crew that sees the launch
//! may fire its smoke dischargers, which is how smoke defeats designation.

use crate::components::{Health, Kin};
use crate::sim::Sim;
use bevy_ecs::prelude::Resource;
use bgws_core::{figure, Enu, EntityId, Figure, Figures, Side, SimTime};
use serde::Serialize;

pub fn figures() -> Vec<Figure> {
    vec![
        figure!("laser.lost_cep_m", 50.0, "m", 5.0, 500.0, "default", Default, "miss distance (CEP) of a semi-active laser munition that has lost its spot"),
        figure!("laser.min_transmission", 0.1, "share", 0.001, 0.9, "default", Default, "two-way near-infrared transmission below which the seeker cannot see the spot"),
        figure!("laser.min_p_visible", 0.5, "p", 0.05, 1.0, "default", Default, "for a designator in the air: chance of a clear line over the ground below which it has lost the target"),
        figure!("laser.code", 1688.0, "code", 1111.0, 1788.0, "default", Default, "the laser code designators set (a NATO-style four-digit code)"),
    ]
}

/// One designation: a designator keeping a spot on a target for a munition.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Designation {
    pub id: u64,
    pub side: Side,
    pub designator: EntityId,
    pub target: Option<EntityId>,
    /// The munition guided on it.
    pub munition: EntityId,
    /// Where the spot is (east, north, height above mean sea level).
    pub spot: Enu,
    pub code: u16,
    pub since: SimTime,
    pub until: SimTime,
    /// The spot is on the target and the seeker can see it.
    pub lasing: bool,
}

#[derive(Resource, Default)]
pub struct Designations {
    pub active: Vec<Designation>,
    next: u64,
}

fn fig(s: &Sim, k: &str) -> f64 {
    let f: &Figures = &s.ctx.figures;
    f.get(k)
}

/// Is a munition semi-active laser (it needs a designator)?
pub fn is_sal(m: &bgws_catalogue::Munition) -> bool {
    m.guidance == bgws_catalogue::Guidance::Laser && m.seeker.as_ref().is_some_and(|x| x.kind == "laser")
}

pub fn setup(s: &mut Sim) {
    s.world.insert_resource(Designations::default());
}

/// A semi-active laser munition is launched: its designator starts lasing.
pub fn start(s: &mut Sim, side: Side, designator: EntityId, target: Option<EntityId>, munition: EntityId, aim: Enu, until: SimTime) {
    let now = s.ctx.now;
    let code = fig(s, "laser.code").round() as u16;
    let id = {
        let Some(mut d) = s.world.get_resource_mut::<Designations>() else { return };
        d.next += 1;
        let id = d.next;
        d.active.push(Designation { id, side, designator, target, munition, spot: aim, code, since: now, until, lasing: true });
        id
    };
    let text = format!("{} lases {} for its missile", crate::systems::name(s, designator), target.map(|t| crate::systems::name(s, t)).unwrap_or_else(|| "its aim point".into()));
    let mut ev = s.ctx.log.push(now, "laser.designate").side(side).entity(designator).text(text).data(serde_json::json!({ "designation": id, "code": code }));
    if let Some(t) = target {
        ev = ev.target(t);
    }
    drop(ev);
    update(s);
}

/// Can the designator keep its spot on the target now?
fn lasing(s: &Sim, d: &Designation) -> (bool, Enu) {
    let ent = |id: EntityId| s.ctx.order.get(id.index()).copied();
    let Some(de) = ent(d.designator) else { return (false, d.spot) };
    if s.world.get::<Health>(de).is_none_or(|h| h.destroyed) {
        return (false, d.spot);
    }
    let Some(dk) = s.world.get::<Kin>(de).copied() else { return (false, d.spot) };
    let Some(tk) = d.target.and_then(ent).and_then(|e| s.world.get::<Kin>(e).copied()) else { return (true, d.spot) };
    let spot = Enu::new(tk.pos.e, tk.pos.n, tk.pos.u + 1.0);
    // A designator on the ground lases what it has in its sight now (as a
    // gunner guides a missile); an aircraft, what it can see from up there.
    let in_sight = match s.world.get::<crate::components::Aware>(de).and_then(|a| d.target.and_then(|t| a.seen.get(&t))) {
        // Sightings refresh every few seconds; its tracker holds between.
        Some(seen) if dk.agl < 5.0 => (s.ctx.now - seen.last).secs() <= 5.0,
        _ if dk.agl < 5.0 => false,
        _ => s.ctx.ground.los(dk.pos, dk.agl, tk.pos, 1.0 + tk.agl).p_visible >= fig(s, "laser.min_p_visible"),
    };
    if !in_sight {
        return (false, spot);
    }
    let t = s.world.get_resource::<crate::smoke::SmokeState>().map_or(1.0, |sm| sm.transmission(bgws_models::signature::Band::NearIr, Enu::new(dk.pos.e, dk.pos.n, dk.agl.max(1.5)), Enu::new(tk.pos.e, tk.pos.n, 1.0 + tk.agl), s.ctx.now));
    // There to the target, and back to the seeker on much the same line.
    (t * t >= fig(s, "laser.min_transmission"), spot)
}

/// 1 Hz: each spot follows its target; a designation that loses it says so.
pub fn update(s: &mut Sim) {
    let now = s.ctx.now;
    let Some(list) = s.world.get_resource::<Designations>().map(|d| d.active.clone()) else { return };
    let mut out = Vec::with_capacity(list.len());
    for mut d in list {
        // Its munition ends it on landing; this only clears one that never did.
        if (now - d.until).secs() > 30.0 {
            continue;
        }
        let (on, spot) = lasing(s, &d);
        if d.lasing && !on {
            let text = format!("{} loses its laser spot on {}", crate::systems::name(s, d.designator), d.target.map(|t| crate::systems::name(s, t)).unwrap_or_default());
            s.ctx.log.push(now, "laser.lost").side(d.side).entity(d.designator).text(text).data(serde_json::json!({ "designation": d.id }));
        }
        d.lasing = on;
        d.spot = spot;
        out.push(d);
    }
    if let Some(mut st) = s.world.get_resource_mut::<Designations>() {
        st.active = out;
    }
}

/// The munition lands: does it still have its spot? The designation ends.
pub fn finish(s: &mut Sim, munition: EntityId) -> Option<Designation> {
    update(s);
    let mut st = s.world.get_resource_mut::<Designations>()?;
    let k = st.active.iter().position(|d| d.munition == munition)?;
    Some(st.active.remove(k))
}

/// The miss distance a semi-active laser munition lands with when it has
/// lost its spot.
pub fn lost_cep_m(s: &Sim) -> f64 {
    fig(s, "laser.lost_cep_m")
}
