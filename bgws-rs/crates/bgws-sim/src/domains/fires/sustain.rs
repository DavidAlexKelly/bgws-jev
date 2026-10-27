//! Unit stores and consumption (§8.11, the TypeScript `landExt.ts`
//! sustainment): fuel burnt by the kilometre and the idle hour
//! (`bgws_ops::logistics::consumption`, Abrams-class figures **[P17]**),
//! ammunition by every round fired (the mounts), food and water by the
//! person-day. A unit that runs low asks its leader (`stocksLow`); with no
//! fuel it stops until resupplied. With logistics off, stores are not drawn.

use super::{alive, name, raise, FiresWorld, Stores, Tally};
use crate::components::*;
use crate::sim::Sim;
use bevy_ecs::prelude::Entity;
use bgws_catalogue::{Domain, MoverKind, Platform, Vulnerability};
use bgws_core::{EntityId, SimDuration};
use bgws_ops::logistics::{consumption, Activity as LogActivity, Terrain, VehicleClass};
use bgws_scenario::scope::DomainMode;

/// The consumption class of a land platform (None: it burns no fuel).
pub fn vehicle_class(p: &Platform) -> Option<VehicleClass> {
    match p.mover.kind {
        MoverKind::Tracked | MoverKind::Wheeled => {
            let c = format!("{} {}", p.class, p.role).to_lowercase();
            if c.contains("mbt") || c.contains("tank") || p.body.mass_t >= 40.0 {
                Some(VehicleClass::Mbt)
            } else if matches!(p.body.vulnerability, Vulnerability::Armoured | Vulnerability::Light) {
                Some(VehicleClass::Ifv)
            } else {
                Some(VehicleClass::Truck)
            }
        }
        _ => None,
    }
}

pub fn setup(s: &mut Sim, st: &FiresWorld) {
    let food_pd = s.ctx.figures.get("log.food_kg_per_person_day");
    let water_pd = s.ctx.figures.get("log.water_l_per_person_day");
    for &e in &s.ctx.order.clone() {
        let Some(i) = s.world.get::<Ident>(e) else { continue };
        let p = i.platform.clone();
        let h = s.world.get::<Health>(e).expect("h");
        let tally = Tally { fit: h.fit, mobility: h.mobility_kills, firepower: h.firepower_kills, destroyed: h.destroyed };
        let n = h.start;
        let land = i.domain == Domain::Land && !super::is_node(&p);
        s.world.entity_mut(e).insert(tally);
        if !land {
            continue;
        }
        let arms = s.world.get::<Arms>(e).expect("arms");
        let ammo_max = arms.mounts.iter().map(|m| (m.index, m.rounds)).collect();
        let per = (p.people() as f64 / p.body.count.max(1) as f64).max(1.0);
        let people = (per * n as f64).round() as u32;
        let class = vehicle_class(&p);
        let fuel_max = if class.is_some() { p.mover.fuel_capacity_l * n as f64 } else { 0.0 };
        let food = people as f64 * food_pd * st.p.food_days;
        let water = people as f64 * water_pd * st.p.food_days;
        let pos = s.world.get::<Kin>(e).expect("k").pos;
        s.world.entity_mut(e).insert(Stores {
            fuel_l: fuel_max,
            fuel_max_l: fuel_max,
            ammo_max,
            food_kg: food,
            food_max_kg: food,
            water_l: water,
            water_max_l: water,
            people,
            class,
            low_since: None,
            stalled: None,
            last_pos: pos,
            op_hours: 0.0,
            carrier_t: p.carried.cargo_t,
        });
    }
}

/// Fuel burn, running dry, and the low-stocks question, each second.
pub fn base(s: &mut Sim, st: &mut FiresWorld, dt: f64) {
    if st.logistics_mode == DomainMode::Off {
        return;
    }
    let now = s.ctx.now;
    let figs = s.ctx.figures.clone();
    for &e in &s.ctx.order.clone() {
        let Some(mut stores) = s.world.get::<Stores>(e).cloned() else { continue };
        if !alive(s, e) {
            continue;
        }
        let (id, side) = {
            let i = s.world.get::<Ident>(e).expect("i");
            (i.id, i.side)
        };
        let pos = s.world.get::<Kin>(e).expect("k").pos;
        let fit = s.world.get::<Health>(e).expect("h").fit as f64;
        let act = s.world.get::<Act>(e).expect("a").activity.clone();
        let engine = s.world.get::<SigState>(e).map(|x| x.engine_on).unwrap_or(false);
        let km = pos.sub(stores.last_pos).horizontal() / 1000.0;
        stores.last_pos = pos;
        let moving = km > 1e-4;
        if let Some(class) = stores.class {
            let idle_h = if !moving && engine && act != Activity::Quiet { dt / 3600.0 } else { 0.0 };
            let a = LogActivity { km: vec![(Terrain::CrossCountry, km)], idle_h, ..Default::default() };
            let c = consumption(class, &a, &figs);
            stores.fuel_l = (stores.fuel_l - c.fuel_l * fit).max(0.0);
            stores.op_hours += if moving { dt / 3600.0 } else { idle_h * st.p.idle_engine_share };
        }
        // Out of fuel: it stops where it is.
        if stores.fuel_max_l > 0.0 && stores.fuel_l <= 0.0 && act.moving() && stores.stalled.is_none() {
            stores.stalled = Some(act.clone());
            s.world.get_mut::<Act>(e).expect("a").activity = Activity::Hold;
            let text = format!("{} is out of fuel and stops", name(s, id));
            s.ctx.log.push(now, "logistics.outOfFuel").side(side).entity(id).text(text);
        } else if stores.fuel_max_l > 0.0 && stores.fuel_l > 0.0 {
            if let Some(a) = stores.stalled.take() {
                s.world.get_mut::<Act>(e).expect("a").activity = a;
            }
        }
        // Low: ask the leader, once until resupplied.
        let ammo = stores.ammo_pct(s.world.get::<Arms>(e).expect("arms"));
        let fuel = stores.fuel_pct();
        let low = (stores.ammo_max.values().sum::<u32>() > 0 && ammo < st.p.ammo_low) || (stores.fuel_max_l > 0.0 && fuel < st.p.fuel_low);
        let steady = s.world.get::<Crew>(e).map(|c| c.morale.state == bgws_models::people::MoraleState::Steady).unwrap_or(false);
        let ask = low && stores.low_since.is_none() && steady && stores.carrier_t <= 0.0;
        if ask {
            stores.low_since = Some(now);
        } else if !low && stores.low_since.is_some() {
            stores.low_since = None;
        }
        *s.world.get_mut::<Stores>(e).expect("stores") = stores;
        if ask {
            let text = format!("{} reports stocks low (ammunition {:.0}%, fuel {:.0}%)", name(s, id), ammo * 100.0, fuel * 100.0);
            s.ctx.log.push(now, "logistics.stocksLow").side(side).entity(id).text(text).data(serde_json::json!({ "ammoPct": ammo * 100.0, "fuelPct": fuel * 100.0 }));
            let rule = if ammo.min(fuel) < st.p.withdraw_below { "withdraw" } else { "requestResupply" };
            if !raise(s, id, "stocksLow", None, Some(rule)) {
                request(s, st, id);
            }
        }
    }
}

/// Food and water by the person-day, each minute.
pub fn minute(s: &mut Sim, st: &mut FiresWorld, dt: f64) {
    if st.logistics_mode == DomainMode::Off {
        return;
    }
    let food = s.ctx.figures.get("log.food_kg_per_person_day");
    let water = s.ctx.figures.get("log.water_l_per_person_day");
    for &e in &s.ctx.order.clone() {
        if !alive(s, e) {
            continue;
        }
        let Some(mut st) = s.world.get_mut::<Stores>(e) else { continue };
        let days = st.people as f64 * dt / 86_400.0;
        st.food_kg = (st.food_kg - days * food).max(0.0);
        st.water_l = (st.water_l - days * water).max(0.0);
    }
}

/// Tonnes of ammunition (class V) and fuel (class III) a unit needs to be full.
pub fn need(s: &Sim, st: &FiresWorld, e: Entity) -> (f64, f64) {
    let (Some(stores), Some(arms), Some(i)) = (s.world.get::<Stores>(e), s.world.get::<Arms>(e), s.world.get::<Ident>(e)) else { return (0.0, 0.0) };
    let mut v = 0.0;
    for m in &arms.mounts {
        let max = stores.ammo_max.get(&m.index).copied().unwrap_or(0);
        if max > m.rounds {
            let fam = s.ctx.catalogue.munition(&m.munition).map(|x| x.family.clone()).unwrap_or_default();
            v += (max - m.rounds) as f64 * st.p.kg_per_round(&fam) / 1000.0;
        }
    }
    let _ = i;
    let iii = (stores.fuel_max_l - stores.fuel_l).max(0.0) * st.p.fuel_kg_per_l / 1000.0;
    (v, iii)
}

/// Hand a unit tonnes of ammunition and fuel: rounds go to each mount in
/// proportion to what it lacks. Returns (rounds added, litres added).
pub fn refill(s: &mut Sim, st: &FiresWorld, e: Entity, v_t: f64, iii_t: f64) -> (u32, f64) {
    let (need_v, _) = need(s, st, e);
    let share = if need_v > 0.0 { (v_t / need_v).min(1.0) } else { 0.0 };
    let mut added = 0;
    let max = s.world.get::<Stores>(e).map(|x| x.ammo_max.clone()).unwrap_or_default();
    if let Some(mut arms) = s.world.get_mut::<Arms>(e) {
        for m in arms.mounts.iter_mut() {
            let cap = max.get(&m.index).copied().unwrap_or(0);
            if cap > m.rounds {
                let n = ((cap - m.rounds) as f64 * share).floor() as u32;
                m.rounds += n;
                added += n;
            }
        }
    }
    let mut litres = 0.0;
    if let Some(mut stores) = s.world.get_mut::<Stores>(e) {
        litres = (iii_t * 1000.0 / st.p.fuel_kg_per_l).min(stores.fuel_max_l - stores.fuel_l).max(0.0);
        stores.fuel_l += litres;
        // Food and water ride with every delivery.
        stores.food_kg = stores.food_max_kg;
        stores.water_l = stores.water_max_l;
        stores.low_since = None;
    }
    (added, litres)
}

/// A unit asks for resupply: the network takes it (simulated), or an
/// off-board service meets it after its delay (abstracted).
pub fn request(s: &mut Sim, st: &mut FiresWorld, id: EntityId) {
    let now = s.ctx.now;
    let e = s.ctx.entity(id);
    let side = s.world.get::<Ident>(e).expect("i").side;
    match st.logistics_mode {
        DomainMode::Off => return,
        DomainMode::Simulated => {
            if !st.logistics.requests.entry(side).or_default().insert(id) {
                return;
            }
        }
        DomainMode::Abstracted => {
            let (v, iii) = need(s, st, e);
            let at = now + SimDuration::from_mins(st.p.abstract_resupply_min);
            super::logistics::schedule_delivery(s, st, super::logistics::DeliverTo::Unit(id), v, iii, at, 0.0);
        }
    }
    let text = format!("{} asks for resupply", name(s, id));
    s.ctx.log.push(now, "logistics.request").side(side).entity(id).text(text);
}

/// `land.requestResupply`, `land.conserve`, `land.withdrawToResupply`.
pub fn apply(s: &mut Sim, st: &mut FiresWorld, e: Entity, id: EntityId, action: &str) -> bool {
    match action {
        "land.requestResupply" => request(s, st, id),
        "land.conserve" => {
            if let Some(mut a) = s.world.get_mut::<Act>(e) {
                a.roe = RoeS::IfFiredUpon;
            }
        }
        "land.withdrawToResupply" => {
            let side = s.world.get::<Ident>(e).expect("i").side;
            let pos = s.world.get::<Kin>(e).expect("k").pos;
            let node = super::logistics::nearest_node(s, st, side, pos);
            let to = node.and_then(|n| s.world.get::<Kin>(s.ctx.entity(n)).map(|k| k.pos)).unwrap_or_else(|| {
                let away = crate::systems::threat_bearing(s, e).map(|b| b + std::f64::consts::PI).unwrap_or(if side == bgws_core::Side::Blue { std::f64::consts::PI } else { 0.0 });
                pos.add(bgws_core::Enu::new(away.sin(), away.cos(), 0.0).scale(2.0 * s.ctx.p.withdraw_m))
            });
            let route = crate::decide::route_for(s, e, to);
            if let Some(mut a) = s.world.get_mut::<Act>(e) {
                a.activity = Activity::Withdraw { route, then: Some(Box::new(Activity::Overwatch)) };
            }
            request(s, st, id);
        }
        _ => return false,
    }
    true
}
