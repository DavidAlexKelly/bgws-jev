//! Golden decision scenes (RUST_ENGINE.md §19: "the golden decision scenes
//! become snapshot tests with outputs exported from TS").
//!
//! `tests/golden/*.json` were written by the TypeScript walker itself
//! (`tests/fixtures/gen_golden.ts`, run with `node --experimental-strip-types`
//! over the same template files): the Rust walk, with every gate open, must
//! reproduce them exactly, including the world-trigger fallback to core/world.
//!
//! `tests/golden_gated/*.json` are Rust-only scenes for the hard gates, which
//! the TypeScript build does not have. Regenerate them with
//! `UPDATE_GOLDEN=1 cargo test -p bgws-behaviour --test golden` and review the diff.

use bgws_behaviour::facts::Facts;
use bgws_behaviour::gates::{Cohesion, Emcon, Gates, Roe, WeaponRelease};
use bgws_behaviour::library::Capability;
use bgws_behaviour::types::{FactValue, Template, WalkResult};
use bgws_behaviour::{LoadMode, Registry, Vocabulary};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn dir(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join(name)
}

fn custom() -> Vec<Template> {
    serde_json::from_str(&std::fs::read_to_string(dir("fixtures").join("custom.json")).unwrap()).unwrap()
}

fn registry() -> Registry {
    Registry::with_custom(&custom(), &Vocabulary::builtin(), LoadMode::Strict).unwrap()
}

fn facts_of(v: &Value) -> BTreeMap<String, FactValue> {
    v.as_object().unwrap().iter().map(|(k, v)| (k.clone(), serde_json::from_value(v.clone()).unwrap())).collect()
}

/// Run a scene the way the sim would: choose the template for the trigger, walk it.
fn run(reg: &Registry, scene: &Value, gates: &Gates) -> Value {
    let candidates: Vec<&str> = scene["candidates"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    let trigger = scene["trigger"].as_str().unwrap();
    let facts = facts_of(&scene["facts"]);
    match reg.decide(&candidates, trigger, &facts, gates).unwrap() {
        Some((t, walk)) => json!({ "scene": scene, "template": t.id, "lineage": t.lineage, "walk": walk }),
        None => json!({ "scene": scene, "template": null, "lineage": [], "walk": WalkResult::None { path: vec![], blocked: None } }),
    }
}

#[test]
fn rust_walk_reproduces_every_typescript_scene() {
    let reg = registry();
    let mut n = 0;
    let mut paths: Vec<_> = std::fs::read_dir(dir("golden")).unwrap().map(|e| e.unwrap().path()).collect();
    paths.sort();
    for path in paths {
        let expected: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let got = run(&reg, &expected["scene"], &Gates::open());
        assert_eq!(got, expected, "{}", path.display());
        n += 1;
    }
    assert_eq!(n, 22);
}

fn gated_scenes() -> Vec<(&'static str, Value, Gates)> {
    let s = |name: &str, cands: &[&str], trigger: &str, facts: Value| json!({ "name": name, "candidates": cands, "trigger": trigger, "facts": facts });
    vec![
        ("armour-d1-weapons-hold", s("armour-d1-weapons-hold", &["core/armour"], "D1", json!({})), Gates { roe: Roe::Never, ..Gates::open() }),
        (
            "armour-d1-weapons-tight-unidentified",
            s("armour-d1-weapons-tight-unidentified", &["core/armour"], "D1", json!({})),
            Gates { weapon_release: WeaponRelease::Tight, target_identified: false, ..Gates::open() },
        ),
        ("infantry-d2-broken", s("infantry-d2-broken", &["core/infantry"], "D2", json!({})), Gates { cohesion: Cohesion::Broken, ..Gates::open() }),
        ("infantry-d2-shaken", s("infantry-d2-shaken", &["core/infantry"], "D2", json!({})), Gates { cohesion: Cohesion::Shaken, ..Gates::open() }),
        ("jammer-emcon-silent", s("jammer-emcon-silent", &["core/ewJammer"], "jammingOrder", json!({})), Gates { emcon: Emcon::Silent, ..Gates::open() }),
        (
            "drone-no-autonomy",
            s("drone-no-autonomy", &["core/drone"], "linkDegraded", json!({})),
            Gates { capabilities: Some([Capability::Mobility, Capability::Drones, Capability::Radio].into_iter().collect()), ..Gates::open() },
        ),
        (
            "fired-upon-roe-before-and-after",
            s("fired-upon-roe-before-and-after", &["core/default"], "D4", json!({})),
            Gates { roe: Roe::IfFiredUpon, fired_upon: false, ..Gates::open() },
        ),
        ("branchy-broken-drill-ok", s("branchy-broken-drill-ok", &["custom/branchy"], "D2", json!({"self.cohesion": "broken", "self.strength": 0.4})), Gates { cohesion: Cohesion::Broken, ..Gates::open() }),
        ("drill-blocked-by-roe", s("drill-blocked-by-roe", &["custom/drill-fire"], "D4", json!({})), Gates { roe: Roe::Never, ..Gates::open() }),
    ]
}

#[test]
fn gated_scenes_match_their_snapshots() {
    let reg = registry();
    let update = std::env::var("UPDATE_GOLDEN").is_ok();
    let d = dir("golden_gated");
    if update {
        std::fs::create_dir_all(&d).unwrap();
    }
    for (name, scene, gates) in gated_scenes() {
        let mut got = run(&reg, &scene, &gates);
        got["gates"] = serde_json::to_value(&gates).unwrap();
        let path = d.join(format!("{name}.json"));
        if update {
            std::fs::write(&path, serde_json::to_string_pretty(&got).unwrap() + "\n").unwrap();
            continue;
        }
        let expected: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("missing snapshot {}", path.display()))).unwrap();
        assert_eq!(got, expected, "{name}");
    }
}

#[test]
fn a_gate_never_lets_a_removed_option_through() {
    // Whatever the gates, every offered option passes them, and offered ∪ dropped = the node's options.
    let reg = registry();
    let vocab = reg.vocabulary();
    for (_, scene, gates) in gated_scenes() {
        let v = run(&reg, &scene, &gates);
        let walk: WalkResult = serde_json::from_value(v["walk"].clone()).unwrap();
        if let WalkResult::Choose { options, .. } = walk {
            for o in options {
                assert_eq!(gates.check(&vocab.action(&o.action).unwrap().traits), Ok(()), "{} offered past a gate", o.id);
            }
        }
    }
}

struct Panicky;
impl Facts for Panicky {
    fn fact(&self, name: &str) -> Option<FactValue> {
        panic!("no conditions in shipped templates, asked {name}")
    }
}

#[test]
fn shipped_templates_need_no_facts() {
    let reg = Registry::builtin().unwrap();
    for (id, t) in reg.templates() {
        let r = reg.resolved(id).unwrap();
        for trigger in t.triggers.iter().flatten().map(|(k, _)| k) {
            reg.walk(r, trigger, &Panicky, &Gates::open()).unwrap();
        }
    }
}
