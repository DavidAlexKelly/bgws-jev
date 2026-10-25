//! A port of `behaviour/actions/land.test.ts` and the template-level checks of
//! `behaviour/inPlay.test.ts`: the default land template runs, at every
//! decision point, the actions that point has always used, in order; variants
//! remove, condition and re-bias exactly what they name; unknown or invalid
//! templates fall back to the built-in ones; type defaults carry doctrine.

use bgws_behaviour::archetypes::{default_template_for_class, DEFAULT_TEMPLATE_ID};
use bgws_behaviour::facts::{fact_map, NoFacts};
use bgws_behaviour::library::land_action_names;
use bgws_behaviour::types::{Comparison, Condition, Template, WalkResult};
use bgws_behaviour::{Gates, LoadMode, Registry, Vocabulary};

const POINTS: [&str; 13] = ["D0", "D1", "D2", "D3", "D4", "D5", "D6", "D7", "D8", "D9", "D10", "D11", "D12"];

fn plan(reg: &Registry, id: &str, point: &str) -> Vec<String> {
    let r = reg.resolved_or_default(id).unwrap();
    match reg.walk(r, point, &NoFacts, &Gates::open()).unwrap() {
        WalkResult::Choose { options, .. } => options.into_iter().map(|o| o.action).collect(),
        _ => vec![],
    }
}

#[test]
fn the_default_land_template_runs_each_points_actions_in_order() {
    let reg = Registry::builtin().unwrap();
    let split: [(&str, &[&str]); 13] = [
        ("D0", &["land.complyWithOrders", "land.breakOffCovered", "land.fireAndBack", "land.finishFightFirst", "land.breakContactFirst"]),
        ("D1", &["land.carryOn", "land.engageNow", "land.waitFor", "land.betterPosition", "land.observe", "land.pullBackToFriends"]),
        ("D2", &["land.returnFire", "land.hullDownThenFire", "land.outOfSight", "land.askFriendForHelp", "land.assault", "land.goQuietInCover"]),
        ("D3", &["land.searchBearing", "land.coverFromBearing", "land.pullBackFromBearing", "land.carryOn"]),
        ("D4", &["land.fireNow", "land.waitCloser", "land.letPass"]),
        ("D5", &["land.fireFirst", "land.keepHolding", "land.relocateQuietly", "land.slipAway"]),
        ("D6", &["land.keepFiring", "land.fireAndMove", "land.shiftFire", "land.goQuiet", "land.outOfSight"]),
        ("D7", &["land.resumeOrCarryOn", "land.watchLastSeen", "land.regainContact", "land.shiftFireFromLost"]),
        ("D8", &["land.betterShotOnTarget", "land.shiftFireFromTarget", "land.stopFiring", "land.outOfTargetSight", "land.keepAtIt"]),
        ("D9", &["land.helpFriend", "land.moveToSupport", "land.carryOn"]),
        ("D10", &["land.resumeOrders", "land.skipToNextPhase", "land.holdHere", "land.overwatchHere", "land.keepHoldingHere"]),
        ("D11", &["land.rejoin", "land.holdAndRecover", "land.joinNearestFriends"]),
        ("D12", &["land.complyWithRequest", "land.partlyOverwatch", "land.cannotComply"]),
    ];
    for (point, want) in split {
        assert_eq!(plan(&reg, DEFAULT_TEMPLATE_ID, point), want, "{point}");
    }
}

#[test]
fn a_variant_drops_one_option_at_d1_stand_fast_removes_pulling_back() {
    let sf = Template { remove: Some(vec!["pullBack".into()]), ..Template::variant("custom/stand-fast", "core/default") };
    let reg = Registry::with_custom(&[sf], &Vocabulary::builtin(), LoadMode::Strict).unwrap();
    assert!(plan(&reg, "core/default", "D1").contains(&"land.pullBackToFriends".to_string()));
    let mut want = plan(&reg, "core/default", "D1");
    want.retain(|a| a != "land.pullBackToFriends");
    assert_eq!(plan(&reg, "custom/stand-fast", "D1"), want);
}

#[test]
fn names_only_actions_that_exist() {
    let reg = Registry::builtin().unwrap();
    let land = land_action_names();
    for p in POINTS {
        for a in plan(&reg, DEFAULT_TEMPLATE_ID, p) {
            assert!(land.contains(&a.as_str()), "{a}");
        }
    }
}

fn ids(reg: &Registry, id: &str, point: &str, facts: &impl bgws_behaviour::Facts) -> Vec<String> {
    let r = reg.resolved_or_default(id).unwrap();
    reg.walk(r, point, facts, &Gates::open()).unwrap().option_ids().into_iter().map(String::from).collect()
}

#[test]
fn stand_fast_is_never_offered_pulling_back_anywhere_and_keeps_the_rest_in_order() {
    let sf = Template { remove: Some(vec!["pullBack".into()]), ..Template::variant("custom/stand-fast", DEFAULT_TEMPLATE_ID) };
    let reg = Registry::with_custom(&[sf], &Vocabulary::builtin(), LoadMode::Strict).unwrap();
    let mut seen = 0;
    for p in POINTS {
        let before = ids(&reg, DEFAULT_TEMPLATE_ID, p, &NoFacts);
        if before.iter().any(|i| i.starts_with("pullBack")) {
            seen += 1;
        }
        let after = ids(&reg, "custom/stand-fast", p, &NoFacts);
        assert!(!after.iter().any(|i| i.starts_with("pullBack")), "{p}");
        assert_eq!(after, before.into_iter().filter(|i| !i.starts_with("pullBack")).collect::<Vec<_>>(), "{p}");
    }
    assert!(seen > 0);
}

#[test]
fn an_only_when_condition_removes_an_option_when_it_does_not_hold() {
    let pb = Template {
        when: Some([("engage".to_string(), Condition::fact("enemy.rangeM", Comparison::Le, 1.0))].into_iter().collect()),
        ..Template::variant("custom/point-blank", DEFAULT_TEMPLATE_ID)
    };
    let reg = Registry::with_custom(&[pb], &Vocabulary::builtin(), LoadMode::Strict).unwrap();
    let facts = fact_map([("enemy.rangeM", 800.0.into())]);
    assert!(ids(&reg, DEFAULT_TEMPLATE_ID, "D1", &facts).contains(&"engage".into()));
    assert!(!ids(&reg, "custom/point-blank", "D1", &facts).contains(&"engage".into()));
    let close = fact_map([("enemy.rangeM", 1.0.into())]);
    assert!(ids(&reg, "custom/point-blank", "D1", &close).contains(&"engage".into()));
}

#[test]
fn a_preference_travels_with_its_option_only() {
    let wary = Template { bias: Some([("observe".to_string(), -2)].into_iter().collect()), ..Template::variant("custom/wary", DEFAULT_TEMPLATE_ID) };
    let reg = Registry::with_custom(&[wary], &Vocabulary::builtin(), LoadMode::Strict).unwrap();
    let r = reg.walk(reg.resolved("custom/wary").unwrap(), "D1", &NoFacts, &Gates::open()).unwrap();
    let WalkResult::Choose { options, .. } = r else { panic!() };
    for o in options {
        assert_eq!(o.bias, if o.id == "observe" { -2 } else { 0 }, "{}", o.id);
    }
}

#[test]
fn an_unknown_or_invalid_template_falls_back_to_the_built_in_behaviour() {
    let broken = Template { remove: Some(vec!["keep".into()]), ..Template::variant("custom/broken", DEFAULT_TEMPLATE_ID) };
    let (reg, err) = Registry::with_custom_or_builtin(&[broken], &Vocabulary::builtin(), LoadMode::Strict).unwrap();
    assert!(err.unwrap().to_string().contains("fallback"));
    assert!(!reg.contains("custom/broken"));
    assert_eq!(ids(&reg, "custom/broken", "D1", &NoFacts), ids(&reg, DEFAULT_TEMPLATE_ID, "D1", &NoFacts));
    assert_eq!(ids(&reg, "custom/missing", "D1", &NoFacts), ids(&reg, DEFAULT_TEMPLATE_ID, "D1", &NoFacts));
}

#[test]
fn units_default_to_their_types_template_which_carries_doctrine() {
    let reg = Registry::builtin().unwrap();
    for class in [Some("armoured_vehicle"), Some("foot"), Some("soft_skin"), None] {
        let id = default_template_for_class(class);
        let doctrine = &reg.resolved(id).unwrap().doctrine;
        assert_eq!(!doctrine.is_empty(), id != DEFAULT_TEMPLATE_ID, "{id}");
    }
    assert_eq!(reg.lineage("core/armour"), ["core/armour", "core/default"]);
}
