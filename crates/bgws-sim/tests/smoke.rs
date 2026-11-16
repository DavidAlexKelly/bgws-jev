//! Smoke and laser designation in the simulation: a screen cuts sight
//! through it (by band); armoured vehicles smoke themselves when a missile
//! is launched at them; units call smoke screens on enemies they cannot
//! answer; a semi-active laser missile keeps its spot over a clear line and
//! loses it to smoke (the target's own or a screen).

use bgws_core::{Enu, EntityId, Figures, Seed, Side, SimTime};
use bgws_models::signature::SmokeKind;
use bgws_scenario::v4::ScenarioV4;
use bgws_sim::components::{Aware, Ident, Kin};
use bgws_sim::smoke::Source;
use bgws_sim::{default_figures, Sim};

fn sim(name: &str, f: Figures) -> Sim {
    let path = format!("{}/../../data/scenarios/{name}.json", env!("CARGO_MANIFEST_DIR"));
    Sim::new(&ScenarioV4::from_json(&std::fs::read_to_string(path).unwrap()).unwrap(), Seed(1), f).unwrap()
}

fn pos(s: &Sim, id: EntityId) -> Enu {
    s.world.get::<Kin>(s.ctx.entity(id)).unwrap().pos
}

/// A red unit that sees a blue armoured vehicle now, and that vehicle.
fn seeing_pair(s: &Sim) -> Option<(EntityId, EntityId)> {
    for &e in &s.ctx.order {
        let i = s.world.get::<Ident>(e)?;
        if i.side != Side::Red {
            continue;
        }
        let aw = s.world.get::<Aware>(e)?;
        for (t, seen) in &aw.seen {
            if (s.ctx.now - seen.last).secs() <= 1.0 && bgws_sim::smoke::has_dischargers(s, *t) {
                return Some((i.id, *t));
            }
        }
    }
    None
}

/// Run until some red unit sees a blue armoured vehicle.
fn until_seeing(s: &mut Sim) -> (EntityId, EntityId) {
    for t in 1..=1800 {
        s.run_with_rules(SimTime::from_secs(t as f64));
        if let Some(p) = seeing_pair(s) {
            return p;
        }
    }
    panic!("no red unit saw a blue armoured vehicle");
}

#[test]
fn a_screen_between_two_units_blinds_their_sights() {
    let mut s = sim("gen-company-attack", default_figures());
    let (o, t) = until_seeing(&mut s);
    let before = bgws_sim::systems::explain_detection(&s, o, t, 1.0).unwrap();
    assert_eq!(before["smoke"]["visual"], 1.0);
    // A screen across the line, a third of the way from the target.
    let (po, pt) = (pos(&s, o), pos(&s, t));
    let mid = Enu::new(pt.e + (po.e - pt.e) / 3.0, pt.n + (po.n - pt.n) / 3.0, 0.0);
    let (dx, dy) = ((po.e - pt.e), (po.n - pt.n));
    let len = dx.hypot(dy);
    let pts: Vec<Enu> = [-20.0, -7.0, 7.0, 20.0].iter().map(|k| Enu::new(mid.e - dy / len * k, mid.n + dx / len * k, 0.0)).collect();
    bgws_sim::smoke::release(&mut s, Side::Blue, None, Source::Artillery, SmokeKind::Wp, &pts, 1.0, 20_000.0);
    let after = bgws_sim::systems::explain_detection(&s, o, t, 1.0).unwrap();
    let (vis, ir) = (after["smoke"]["visual"].as_f64().unwrap(), after["smoke"]["thermal"].as_f64().unwrap());
    assert!(vis < 0.05, "visible light through the screen: {vis}");
    assert!(ir > vis, "a thermal sight sees better through phosphorus smoke: {ir} vs {vis}");
}

#[test]
fn vehicles_smoke_themselves_and_units_call_screens() {
    let mut s = sim("m3-battalion-attack", default_figures());
    let mut vehicle = std::collections::BTreeMap::<u32, usize>::new();
    let (mut screens_called, mut artillery) = (0, 0);
    for m in (5..=60).step_by(5) {
        s.run_with_rules(SimTime::from_secs(m as f64 * 60.0));
        for e in s.drain_events() {
            if e.kind == "smoke.screen" {
                match e.data["source"].as_str() {
                    Some("vehicle") => *vehicle.entry(e.entity.unwrap().0).or_default() += 1,
                    Some("artillery") => artillery += 1,
                    _ => {}
                }
            }
            if e.kind == "fires.request" && e.text.contains("screen") {
                screens_called += 1;
            }
        }
    }
    eprintln!("vehicle screens {vehicle:?}; screens called {screens_called}; artillery smoke salvos {artillery}");
    assert!(!vehicle.is_empty(), "no vehicle fired its dischargers");
    assert!(vehicle.values().all(|&n| n <= 2), "no vehicle fires more salvos than it carries");
    for id in vehicle.keys() {
        assert!(bgws_sim::smoke::has_dischargers(&s, EntityId(*id)));
    }
    assert!(screens_called > 0 && artillery > 0, "a screen was called and smoke rounds landed");
}

/// Launch a Hellfire-class missile from the red unit at the blue vehicle;
/// the impact's miss distance and whether the spot was lost.
fn agm_at(s: &mut Sim, designator: EntityId, target: EntityId, smoke_first: bool) -> (f64, bool) {
    let (from, to) = (pos(s, designator), pos(s, target));
    if smoke_first {
        // A screen right in front of the target, across the designator's line.
        let (dx, dy) = (from.e - to.e, from.n - to.n);
        let len = dx.hypot(dy);
        let c = Enu::new(to.e + dx / len * 30.0, to.n + dy / len * 30.0, 0.0);
        let pts: Vec<Enu> = [-15.0, -5.0, 5.0, 15.0].iter().map(|k| Enu::new(c.e - dy / len * k, c.n + dx / len * k, 0.0)).collect();
        bgws_sim::smoke::release(s, Side::Blue, Some(target), Source::Vehicle, SmokeKind::Wp, &pts, 2.0, 2500.0);
    }
    let spec = bgws_sim::domains::air::LaunchSpec {
        side: Side::Red,
        launcher: Some(designator),
        munition: "agm".into(),
        from: Enu::new(from.e, from.n, from.u + 10.0),
        to,
        target: Some(target),
        purpose: bgws_sim::domains::air::Purpose::Strike,
        track: None,
        pk: 0.75,
    };
    bgws_sim::domains::air::flight::launch_munition(s, spec).expect("launched");
    let start = s.ctx.now.secs();
    let (mut miss, mut lost) = (None, false);
    for k in 1..=60 {
        s.run_with_rules(SimTime::from_secs(start + k as f64));
        for e in s.drain_events() {
            lost |= e.kind == "laser.lost";
            if e.kind == "munition.impact" && e.data["munition"] == "agm" {
                miss = e.data["missM"].as_f64();
            }
        }
        if miss.is_some() {
            break;
        }
    }
    (miss.expect("it landed"), lost)
}

#[test]
fn a_laser_guided_missile_keeps_its_spot_unless_smoke_hides_the_target() {
    // No dischargers here: the target cannot smoke itself.
    let mut f = default_figures();
    f.set("smoke.vehicle.salvos", 0.0).unwrap();
    let mut clear = sim("gen-company-attack", f.clone());
    let (o, t) = until_seeing(&mut clear);
    let (miss_clear, lost_clear) = agm_at(&mut clear, o, t, false);
    let mut smoked = sim("gen-company-attack", f);
    let (o2, t2) = until_seeing(&mut smoked);
    assert_eq!((o, t), (o2, t2));
    let (miss_smoked, lost_smoked) = agm_at(&mut smoked, o, t, true);
    eprintln!("clear: miss {miss_clear:.1} m, lost {lost_clear}; smoked: miss {miss_smoked:.1} m, lost {lost_smoked}");
    assert!(!lost_clear && miss_clear < 10.0, "over a clear line it lands on the target");
    assert!(lost_smoked, "smoke across the line loses the spot");
    assert!(miss_smoked > miss_clear);
}

#[test]
fn a_target_that_sees_the_launch_smokes_itself_and_the_spot_is_lost() {
    let mut f = default_figures();
    f.set("smoke.vehicle.warning_p", 1.0).unwrap();
    let mut s = sim("gen-company-attack", f);
    let (o, t) = until_seeing(&mut s);
    let (_, lost) = agm_at(&mut s, o, t, false);
    let st = s.world.get_resource::<bgws_sim::smoke::SmokeState>().unwrap();
    assert_eq!(st.salvos_fired.get(&t), Some(&1), "the target fired one salvo");
    assert!(lost, "its smoke hid it from the designator");
}
