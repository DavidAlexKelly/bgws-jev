//! A port of `behaviour/behaviour.test.ts`: evaluate, walk, inheritance,
//! validation and loading, on the same example templates.

use bgws_behaviour::facts::{fact_map, Facts};
use bgws_behaviour::load::{build_registry, builtin_files, template_id_from_path, TemplateFiles};
use bgws_behaviour::schema::{validate_registry, validate_template, ProblemKind};
use bgws_behaviour::types::{Comparison, Condition, FactValue, Template, WalkResult};
use bgws_behaviour::walk::{evaluate, resolve_template, walk, WalkError};
use bgws_behaviour::{LoadMode, Vocabulary};
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn ctx(ammo: f64, range_m: Option<f64>, suppressed: bool) -> BTreeMap<String, FactValue> {
    fact_map([("ammo.main", ammo.into()), ("target.rangeM", range_m.into()), ("suppressed", suppressed.into())])
}
fn base() -> BTreeMap<String, FactValue> {
    ctx(10.0, Some(1500.0), false)
}

fn tank_json() -> Value {
    json!({
      "id": "core/tank", "version": 1, "archetype": "tank",
      "doctrine": "A tank crew fights from cover and keeps its distance from infantry.",
      "triggers": { "sighted": "contact", "underFire": "underFire" },
      "nodes": {
        "underFire": { "kind": "branch", "cases": [{ "when": { "fact": "suppressed", "op": "==", "value": true }, "next": "smoke" }], "else": "contact" },
        "smoke": { "kind": "do", "action": "popSmokeReverse" },
        "contact": {
          "kind": "choose", "title": "Enemy in sight", "question": "What does the crew do?",
          "options": [
            { "id": "fire", "action": "fire", "when": { "all": [{ "fact": "ammo.main", "op": ">", "value": 0 }, { "fact": "target.rangeM", "op": "<=", "value": 3000 }] } },
            { "id": "hull-down", "action": "takeHullDown" },
            { "id": "wait", "action": "holdObserve", "bias": -1 },
            { "id": "pull-back", "action": "withdraw" }
          ],
          "fallback": "fire"
        }
      }
    })
}
fn stand_fast_json() -> Value {
    json!({ "id": "custom/tank-stand-fast", "version": 1, "parent": "core/tank", "remove": ["pull-back"], "bias": { "wait": 1 } })
}
fn t(v: Value) -> Template {
    serde_json::from_value(v).unwrap()
}
fn registry() -> BTreeMap<String, Template> {
    [t(tank_json()), t(stand_fast_json())].into_iter().map(|t| (t.id.clone(), t)).collect()
}
fn vocab() -> Vocabulary {
    let mut v = Vocabulary::default();
    for f in ["ammo.main", "target.rangeM", "suppressed"] {
        v = v.with_fact(bgws_behaviour::facts::FactInfo { name: f.into(), label: f.into(), kind: bgws_behaviour::facts::FactKind::Number, values: vec![], unit: None });
    }
    for a in ["fire", "takeHullDown", "holdObserve", "withdraw", "popSmokeReverse"] {
        v = v.with_action(bgws_behaviour::library::ActionInfo { name: a.into(), domain: bgws_behaviour::library::Domain::Land, label: a.into(), description: a.into(), traits: Default::default() });
    }
    v
}

#[test]
fn evaluate_compares_combines_and_negates() {
    let gt0 = Condition::fact("ammo.main", Comparison::Gt, 0.0);
    assert!(evaluate(&gt0, &base()).unwrap());
    assert!(!evaluate(&gt0, &ctx(0.0, Some(1500.0), false)).unwrap());
    let any = Condition::Any { any: vec![Condition::fact("suppressed", Comparison::Eq, true), Condition::fact("ammo.main", Comparison::Eq, 10.0)] };
    assert!(evaluate(&any, &base()).unwrap());
    let not = Condition::Not { not: Box::new(Condition::fact("suppressed", Comparison::Eq, false)) };
    assert!(!evaluate(&not, &base()).unwrap());
}

#[test]
fn ordering_with_a_missing_value_is_false_not_zero() {
    assert!(!evaluate(&Condition::fact("target.rangeM", Comparison::Lt, 5000.0), &ctx(10.0, None, false)).unwrap());
    // And text never orders.
    assert!(!evaluate(&Condition::fact("x", Comparison::Gt, 1.0), &fact_map([("x", "9".into())])).unwrap());
}

#[test]
fn refuses_a_fact_it_has_not_been_given() {
    let e = evaluate(&Condition::fact("fuel", Comparison::Gt, 0.0), &base()).unwrap_err();
    assert_eq!(e, WalkError::UnknownFact("fuel".into()));
    assert_eq!(e.to_string(), "Unknown fact \"fuel\"");
}

fn ids(r: &WalkResult) -> Vec<&str> {
    r.option_ids()
}

#[test]
fn walk_offers_options_whose_conditions_hold_keeping_the_fallback() {
    let tank = resolve_template("core/tank", &registry()).unwrap();
    let r = walk(&tank, "sighted", &base()).unwrap();
    assert_eq!(ids(&r), ["fire", "hull-down", "wait", "pull-back"]);
    let WalkResult::Choose { fallback, options, doctrine, path, .. } = &r else { panic!("expected choose") };
    assert_eq!(fallback.as_deref(), Some("fire"));
    assert_eq!(options.iter().find(|o| o.id == "wait").unwrap().bias, -1);
    assert_eq!(doctrine, &vec!["A tank crew fights from cover and keeps its distance from infantry.".to_string()]);
    assert_eq!(path, &vec!["contact".to_string()]);
}

#[test]
fn walk_drops_a_failed_condition_and_falls_back_to_the_first_left() {
    let tank = resolve_template("core/tank", &registry()).unwrap();
    let r = walk(&tank, "sighted", &ctx(0.0, Some(1500.0), false)).unwrap();
    assert_eq!(ids(&r), ["hull-down", "wait", "pull-back"]);
    let WalkResult::Choose { fallback, .. } = &r else { panic!() };
    assert_eq!(fallback.as_deref(), Some("hull-down"));
}

#[test]
fn walk_follows_branches_to_a_drill_or_a_question() {
    let tank = resolve_template("core/tank", &registry()).unwrap();
    let r = walk(&tank, "underFire", &ctx(10.0, Some(1500.0), true)).unwrap();
    assert_eq!(r, WalkResult::Do { node_id: "smoke".into(), action: "popSmokeReverse".into(), params: Default::default(), path: vec!["underFire".into(), "smoke".into()] });
    let r = walk(&tank, "underFire", &base()).unwrap();
    assert_eq!(r.path(), ["underFire", "contact"]);
}

#[test]
fn walk_returns_none_for_a_trigger_it_does_not_handle() {
    let tank = resolve_template("core/tank", &registry()).unwrap();
    assert_eq!(walk(&tank, "lostComms", &base()).unwrap(), WalkResult::None { path: vec![], blocked: None });
}

#[test]
fn walk_stops_a_branch_cycle_instead_of_hanging() {
    let lp = t(json!({
      "id": "core/loop", "version": 1, "triggers": { "sighted": "a" },
      "nodes": {
        "a": { "kind": "branch", "cases": [{ "when": { "fact": "suppressed", "op": "==", "value": false }, "next": "b" }], "else": "b" },
        "b": { "kind": "branch", "cases": [{ "when": { "fact": "suppressed", "op": "==", "value": false }, "next": "a" }], "else": "a" }
      }
    }));
    let reg: BTreeMap<_, _> = [(lp.id.clone(), lp)].into_iter().collect();
    let r = resolve_template("core/loop", &reg).unwrap();
    let e = walk(&r, "sighted", &base()).unwrap_err();
    assert!(e.to_string().contains("exceeded 32 steps"), "{e}");
}

#[test]
fn inheritance_removes_and_rebiases_for_the_child_only() {
    let reg = registry();
    let r = walk(&resolve_template("custom/tank-stand-fast", &reg).unwrap(), "sighted", &base()).unwrap();
    assert_eq!(ids(&r), ["fire", "hull-down", "wait"]);
    let WalkResult::Choose { options, dropped, .. } = &r else { panic!() };
    assert_eq!(options.iter().find(|o| o.id == "wait").unwrap().bias, 1);
    assert_eq!(serde_json::to_value(dropped).unwrap(), json!([{ "id": "pull-back", "action": "withdraw", "why": "removed" }]));
    let base_r = walk(&resolve_template("core/tank", &reg).unwrap(), "sighted", &base()).unwrap();
    assert!(ids(&base_r).contains(&"pull-back"));
}

#[test]
fn inheritance_reports_unknown_parents_and_loops() {
    let orphan = t(json!({ "id": "custom/x", "version": 1, "parent": "core/nope" }));
    let reg: BTreeMap<_, _> = [(orphan.id.clone(), orphan)].into_iter().collect();
    let e = resolve_template("custom/x", &reg).unwrap_err().to_string();
    assert!(e.contains("Unknown template \"core/nope\""), "{e}");
    let a = t(json!({ "id": "custom/a", "version": 1, "parent": "custom/b" }));
    let b = t(json!({ "id": "custom/b", "version": 1, "parent": "custom/a" }));
    let reg: BTreeMap<_, _> = [(a.id.clone(), a), (b.id.clone(), b)].into_iter().collect();
    assert!(resolve_template("custom/a", &reg).unwrap_err().to_string().contains("inheritance loop"));
}

#[test]
fn inheritance_merge_order_child_wins_and_doctrine_is_root_first() {
    let root = t(json!({ "id": "core/r", "version": 1, "doctrine": "root", "bias": {"a": 1, "b": 1}, "remove": ["x"],
        "triggers": {"t": "n"}, "nodes": {"n": {"kind": "choose", "title": "T", "question": "Q", "options": [{"id": "a", "action": "fire"}, {"id": "b", "action": "fire"}, {"id": "x", "action": "fire"}], "fallback": "a"}} }));
    let mid = t(json!({ "id": "custom/m", "version": 1, "parent": "core/r", "doctrine": "mid", "bias": {"a": -2}, "when": {"b": {"fact": "suppressed", "op": "==", "value": true}} }));
    let leaf = t(json!({ "id": "custom/l", "version": 1, "parent": "custom/m", "bias": {"b": 2}, "remove": ["a"] }));
    let reg: BTreeMap<_, _> = [root, mid, leaf].into_iter().map(|t| (t.id.clone(), t)).collect();
    let r = resolve_template("custom/l", &reg).unwrap();
    assert_eq!(r.lineage, ["custom/l", "custom/m", "core/r"]);
    assert_eq!(r.doctrine, ["root", "mid"]);
    assert_eq!(r.bias.get("a"), Some(&-2));
    assert_eq!(r.bias.get("b"), Some(&2));
    assert!(r.remove.contains("x") && r.remove.contains("a"));
    let w = walk(&r, "t", &base()).unwrap();
    // a removed, x removed, b's inherited `when` fails: nothing left, no fallback.
    assert!(ids(&w).is_empty());
    let WalkResult::Choose { fallback, dropped, .. } = &w else { panic!() };
    assert_eq!(*fallback, None);
    assert_eq!(dropped.iter().map(|d| d.why.to_string()).collect::<Vec<_>>(), ["removed", "condition", "removed"]);
}

#[test]
fn validation_accepts_the_examples() {
    let v = vocab();
    assert_eq!(validate_template(&tank_json(), Some(&v)), vec![]);
    assert_eq!(validate_template(&stand_fast_json(), Some(&v)), vec![]);
    assert_eq!(validate_registry(&registry()), vec![]);
}

#[test]
fn validation_lists_every_problem_with_where_it_is() {
    let broken = json!({
      "id": "core/broken", "version": 1, "triggers": { "sighted": "nowhere" },
      "nodes": { "c": { "kind": "choose", "title": "T", "question": "Q", "fallback": "missing",
        "options": [ { "id": "a", "action": "teleport" }, { "id": "a", "action": "fire", "when": { "fact": "morale", "op": "~", "value": 1 } } ] } }
    });
    let errors: Vec<String> = validate_template(&broken, Some(&vocab())).iter().map(|p| p.to_string()).collect();
    for want in [
        "triggers.sighted: no node \"nowhere\"",
        "unknown action \"teleport\"",
        "duplicate option id \"a\"",
        "unknown fact \"morale\"",
        "\"op\" must be one of",
        "fallback \"missing\" is not one of its options",
    ] {
        assert!(errors.iter().any(|e| e.contains(want)), "missing {want:?} in {errors:#?}");
    }
    // Typed, too.
    let kinds: Vec<ProblemKind> = validate_template(&broken, Some(&vocab())).into_iter().map(|p| p.kind).collect();
    assert!(kinds.contains(&ProblemKind::UnknownFact { fact: "morale".into() }));
    assert!(kinds.contains(&ProblemKind::UnknownAction { action: "teleport".into() }));
}

#[test]
fn validation_refuses_a_child_that_removes_its_inherited_fallback() {
    let mut reg = registry();
    let bad = t(json!({ "id": "custom/no-fire", "version": 1, "parent": "core/tank", "remove": ["fire"] }));
    reg.insert(bad.id.clone(), bad);
    let errors: Vec<String> = validate_registry(&reg).iter().map(|p| p.to_string()).collect();
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("removes \"fire\", which is this decision's fallback"), "{errors:?}");
}

#[test]
fn loading_derives_ids_from_paths() {
    assert_eq!(template_id_from_path("./templates/core/tank.json").as_deref(), Some("core/tank"));
    assert_eq!(template_id_from_path("./templates/custom/tank-stand-fast.json").as_deref(), Some("custom/tank-stand-fast"));
    assert_eq!(template_id_from_path("./other/tank.json"), None);
}

#[test]
fn loading_builds_a_registry_and_rejects_a_mismatched_path() {
    let files: TemplateFiles = [("./templates/core/tank.json".to_string(), tank_json()), ("./templates/custom/tank-stand-fast.json".to_string(), stand_fast_json())].into_iter().collect();
    let reg = build_registry(&files, &vocab(), LoadMode::Strict).unwrap();
    assert_eq!(reg.templates().keys().collect::<Vec<_>>(), ["core/tank", "custom/tank-stand-fast"]);
    let bad: TemplateFiles = [("./templates/core/ifv.json".to_string(), tank_json())].into_iter().collect();
    let e = build_registry(&bad, &vocab(), LoadMode::Strict).unwrap_err().to_string();
    assert!(e.contains("path says \"core/ifv\""), "{e}");
    assert!(e.starts_with("Behaviour templates are invalid:\n  "));
}

#[test]
fn every_shipped_template_validates() {
    let reg = build_registry(&builtin_files(), &Vocabulary::builtin(), LoadMode::Strict).unwrap();
    assert_eq!(reg.templates().len(), 26);
    assert!(reg.compatibility().unsupported().is_empty());
}

#[test]
fn the_data_folder_matches_the_compiled_in_set() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/templates");
    let files = bgws_behaviour::load::load_dir(&dir).unwrap();
    let on_disk: BTreeMap<String, Value> = files.into_iter().map(|(p, v)| (template_id_from_path(&p).unwrap(), v)).collect();
    let built: BTreeMap<String, Value> = builtin_files().into_iter().map(|(p, v)| (template_id_from_path(&p).unwrap(), v)).collect();
    assert_eq!(on_disk, built);
}

#[test]
fn facts_trait_objects_work_through_references() {
    let f = base();
    let r: &dyn Facts = &f;
    assert_eq!(r.fact("ammo.main"), Some(FactValue::Num(10.0)));
}
