//! The heuristic planner's fire and movement: with three or more lead units
//! one supports by fire (from an observation point in reach if the terrain
//! analysis gives one) while the others move to a release line and bound
//! onto the objective; with two it attacks as before; a badly hurt lead unit
//! supports by fire from where it is and the reserve takes its place.

use bgws_core::Geodetic;
use bgws_decide::hq::{heuristic_plan, CopTrack, HeuristicParams, HqUnit, TerrainNote, UnitRole};
use bgws_decide::{HqSituation, PlannerKind};
use serde_json::Value;

fn unit(id: &str, lat: f64, lon: f64, fit: u32, reach: f64) -> HqUnit {
    HqUnit { id: id.into(), label: id.into(), role: UnitRole::Manoeuvre, strength: 8.0, vehicles_fit: fit, vehicles_total: 4, at: Geodetic::new(lat, lon, 0.0), reach_m: Some(reach), alive: true }
}

fn situation(units: Vec<HqUnit>, terrain: Vec<TerrainNote>) -> HqSituation {
    HqSituation {
        hq: "B-HQ".into(),
        echelon: "company".into(),
        planner: PlannerKind::Land,
        subordinates: units.iter().map(|u| u.id.clone()).collect(),
        sub_hqs: vec![],
        post: Some(Geodetic::new(54.0, 20.0, 0.0)),
        guidance: None,
        why: "test".into(),
        cop: vec![CopTrack { id: "R1".into(), thought: "tank".into(), domain: "land".into(), seen_ago_s: 10.0, by: "B1".into(), quality: 8, error_m: 50.0, at: Some(Geodetic::new(54.045, 20.0, 0.0)) }],
        units,
        objective: Some(Geodetic::new(54.045, 20.0, 0.0)),
        reference_points: Default::default(),
        shareable_means: Default::default(),
        terrain,
        mission: None,
    }
}

fn params(fm: bool) -> HeuristicParams {
    let mut f = bgws_decide::default_figures();
    f.set("decide.heuristic.fire_and_movement", if fm { 1.0 } else { 0.0 }).unwrap();
    HeuristicParams::from_figures(&f)
}

fn order<'a>(plan: &'a Value, unit: &str) -> &'a Value {
    plan["orders"].as_array().unwrap().iter().find(|o| o["unit"] == unit).unwrap_or_else(|| panic!("no order for {unit}: {plan}"))
}

fn modes(o: &Value) -> Vec<String> {
    o["phases"].as_array().unwrap().iter().map(|p| p["mode"].as_str().unwrap_or(p["do"].as_str().unwrap()).to_string()).collect()
}

#[test]
fn one_gives_fire_while_the_others_bound_in() {
    // Four lead units 5 km short of the objective (B5 the reserve), the longest reach on B2.
    let units = vec![unit("B1", 54.0, 19.99, 4, 3000.0), unit("B2", 54.0, 20.0, 4, 4000.0), unit("B3", 54.0, 20.01, 4, 3000.0), unit("B4", 54.0, 20.02, 4, 3000.0), unit("B5", 53.99, 20.0, 4, 3000.0)];
    let op = TerrainNote { label: "OP1".into(), kind: "observation".into(), at: Geodetic::new(54.025, 20.004, 0.0), note: "sees far".into() };
    let plan = heuristic_plan(&situation(units.clone(), vec![op]), &params(true));
    let sbf = order(&plan, "B2");
    assert_eq!(sbf["task"], "support by fire onto the objective");
    assert!(sbf["phases"][0]["label"].as_str().unwrap().contains("OP1"), "from the observation point: {sbf}");
    assert_eq!(sbf["roe"], "always");
    for u in ["B1", "B3", "B4"] {
        assert_eq!(modes(order(&plan, u)), ["tactical", "bound", "overwatch"], "{u}: release line, bound in, hold");
    }
    assert_eq!(plan["reserve"], "B5");
    // Without fire and movement, as before: everyone advances abreast.
    let old = heuristic_plan(&situation(units, vec![]), &params(false));
    assert!(old["orders"].as_array().unwrap().iter().all(|o| o["task"] != "support by fire onto the objective"));
}

#[test]
fn two_lead_units_attack_together_and_the_hurt_give_fire() {
    // Three manoeuvre units: one is the reserve, two lead. Too few to split.
    let units = vec![unit("B1", 54.0, 19.99, 4, 3000.0), unit("B2", 54.0, 20.0, 4, 3000.0), unit("B3", 53.99, 20.0, 4, 3000.0)];
    let plan = heuristic_plan(&situation(units, vec![]), &params(true));
    assert!(plan["orders"].as_array().unwrap().iter().all(|o| !o["task"].as_str().unwrap().starts_with("support by fire")));
    // Four, one badly hurt: it gives fire from where it is, and the reserve joins the attack.
    let units = vec![unit("B1", 54.0, 19.99, 1, 3000.0), unit("B2", 54.0, 20.0, 4, 3000.0), unit("B3", 54.0, 20.01, 4, 3000.0), unit("B4", 53.99, 20.0, 4, 3000.0)];
    let plan = heuristic_plan(&situation(units, vec![]), &params(true));
    assert_eq!(order(&plan, "B1")["task"], "support by fire: it is too weak to assault");
    assert!(plan.get("reserve").is_none(), "the reserve is committed: {plan}");
    let tasks: Vec<&str> = ["B2", "B3", "B4"].iter().map(|u| order(&plan, u)["task"].as_str().unwrap()).collect();
    assert_eq!(tasks.iter().filter(|t| t.starts_with("support by fire")).count(), 1, "{tasks:?}");
}
