//! A port of `behaviour/store.test.ts` and `behaviour/shared.test.ts`.

use bgws_behaviour::archetypes::DEFAULT_TEMPLATE_ID;
use bgws_behaviour::shared::{publish, pull, Conflict, MemoryRemote};
use bgws_behaviour::store::{slugify, AssignmentWhy, TemplateStore, UnitKind, UnitRef};
use bgws_behaviour::types::Template;
use bgws_core::Side;

fn stand_fast() -> Template {
    Template { name: Some("Stand fast".into()), remove: Some(vec!["pullBack".into()]), ..Template::variant("custom/stand-fast", DEFAULT_TEMPLATE_ID) }
}

#[test]
fn lists_the_built_in_templates_and_saves_a_valid_variant() {
    let mut s = TemplateStore::new();
    assert!(s.all_templates().iter().any(|t| t.id == DEFAULT_TEMPLATE_ID));
    let v0 = s.version();
    assert_eq!(s.save_custom_template(stand_fast(), None), Vec::<String>::new());
    assert!(s.version() > v0, "changes are notified");
    assert_eq!(s.custom_templates().iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["custom/stand-fast"]);
}

#[test]
fn refuses_built_in_ids_and_invalid_templates_storing_nothing() {
    let mut s = TemplateStore::new();
    let builtin = Template { id: "core/default".into(), ..stand_fast() };
    assert!(s.save_custom_template(builtin, None)[0].contains("built-in"));
    // "keep" is D1's fallback: removing it must be refused.
    let no_keep = Template { remove: Some(vec!["keep".into()]), ..stand_fast() };
    assert!(s.save_custom_template(no_keep, None).join("\n").contains("fallback"));
    let orphan = Template { parent: Some("custom/nope".into()), ..stand_fast() };
    assert!(s.save_custom_template(orphan, None).join("\n").contains("Unknown template"));
    assert!(s.custom_templates().is_empty());
}

#[test]
fn assigns_per_asset_falls_back_to_the_default_and_counts_uses() {
    let mut s = TemplateStore::new();
    assert_eq!(s.template_for("m1a2", None), DEFAULT_TEMPLATE_ID);
    s.save_custom_template(stand_fast(), None);
    s.assign_template("m1a2", Some("custom/stand-fast"), None);
    assert_eq!(s.template_for("m1a2", None), "custom/stand-fast");
    assert!(s.is_assigned("m1a2", None));
    assert_eq!(s.assigned_count("custom/stand-fast"), 1);
    s.assign_template("m1a2", None, None);
    assert_eq!(s.template_for("m1a2", None), DEFAULT_TEMPLATE_ID);
}

#[test]
fn moves_assignments_and_children_when_a_template_is_renamed() {
    let mut s = TemplateStore::new();
    s.save_custom_template(stand_fast(), None);
    s.save_custom_template(Template::variant("custom/child", "custom/stand-fast"), None);
    s.assign_template("m1a2", Some("custom/stand-fast"), None);
    let renamed = Template { id: "custom/hold-fast".into(), ..stand_fast() };
    assert_eq!(s.save_custom_template(renamed, Some("custom/stand-fast")), Vec::<String>::new());
    assert_eq!(s.template_for("m1a2", None), "custom/hold-fast");
    assert_eq!(s.custom_templates().iter().find(|t| t.id == "custom/child").unwrap().parent.as_deref(), Some("custom/hold-fast"));
}

#[test]
fn deletes_sending_assets_to_their_default_but_not_while_others_build_on_it() {
    let mut s = TemplateStore::new();
    s.save_custom_template(stand_fast(), None);
    s.save_custom_template(Template::variant("custom/child", "custom/stand-fast"), None);
    s.assign_template("m1a2", Some("custom/stand-fast"), None);
    assert!(s.delete_custom_template("custom/stand-fast")[0].contains("build on it"));
    assert!(s.delete_custom_template("custom/child").is_empty());
    assert!(s.delete_custom_template("custom/stand-fast").is_empty());
    assert_eq!(s.template_for("m1a2", None), DEFAULT_TEMPLATE_ID);
    assert!(s.delete_custom_template(DEFAULT_TEMPLATE_ID)[0].contains("built-in"));
}

#[test]
fn finds_a_free_id_for_a_name() {
    let mut s = TemplateStore::new();
    assert_eq!(s.free_custom_id("Stand Fast!", None), "custom/stand-fast");
    s.save_custom_template(stand_fast(), None);
    assert_eq!(s.free_custom_id("Stand fast", None), "custom/stand-fast-2");
    assert_eq!(s.free_custom_id("Stand fast", Some("custom/stand-fast")), "custom/stand-fast");
    assert_eq!(slugify("  --  "), "behaviour");
    assert_eq!(slugify("Tank: stand fast (night)"), "tank-stand-fast-night");
}

#[test]
fn side_assignments_most_specific_wins() {
    let mut s = TemplateStore::new();
    s.save_custom_template(stand_fast(), None);
    s.save_custom_template(Template { bias: Some([("pullBack".to_string(), 1)].into_iter().collect()), ..Template::variant("custom/cautious", DEFAULT_TEMPLATE_ID) }, None);
    s.assign_template("m1a2", Some("custom/cautious"), Some("armoured_vehicle"));
    s.assign_side(Side::Red, UnitKind::ArmouredVehicle, Some("custom/stand-fast"));
    let tank = |side| UnitRef { id: String::new(), platform: "m1a2".into(), side: Some(side), target_class: Some("armoured_vehicle".into()) };
    // Red's side assignment beats the type's; Blue keeps the type's.
    assert_eq!(s.resolve_behaviour(&tank(Side::Red)), ("custom/stand-fast".into(), AssignmentWhy::Side));
    assert_eq!(s.resolve_behaviour(&tank(Side::Blue)), ("custom/cautious".into(), AssignmentWhy::Asset));
    // Infantry on Red is not armour: its kind's default.
    let rifles = UnitRef { id: "R2".into(), platform: "rifles".into(), side: Some(Side::Red), target_class: Some("foot".into()) };
    assert_eq!(s.resolve_behaviour(&rifles), ("core/infantry".into(), AssignmentWhy::Default));
    assert_eq!(s.side_uses_of("custom/stand-fast"), vec![(Side::Red, UnitKind::ArmouredVehicle)]);

    let snap = s.behaviour_snapshot(&[UnitRef { id: "R1".into(), ..tank(Side::Red) }, UnitRef { id: "B1".into(), ..tank(Side::Blue) }, rifles]);
    assert_eq!(snap.behaviours.into_iter().collect::<Vec<_>>(), [("B1".to_string(), "custom/cautious".to_string()), ("R1".into(), "custom/stand-fast".into())]);
    assert_eq!(snap.why.into_iter().collect::<Vec<_>>(), [("B1".to_string(), AssignmentWhy::Asset), ("R1".into(), AssignmentWhy::Side)]);

    // Cleared: back to the type's.
    s.assign_side(Side::Red, UnitKind::ArmouredVehicle, None);
    assert_eq!(s.resolve_behaviour(&tank(Side::Red)).1, AssignmentWhy::Asset);
}

#[test]
fn the_store_round_trips_as_json() {
    let mut s = TemplateStore::new();
    s.save_custom_template(stand_fast(), None);
    s.assign_side(Side::Blue, UnitKind::Foot, Some("custom/stand-fast"));
    let back: TemplateStore = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    assert_eq!(back, s);
}

// ── shared.test.ts ──────────────────────────────────────────────────────

fn sf() -> Template {
    Template { name: Some("Tank: stand fast".into()), remove: Some(vec!["pullBack".into()]), ..Template::variant("custom/tank-stand-fast", "core/default") }
}
fn child() -> Template {
    Template { name: Some("…at night".into()), ..Template::variant("custom/tank-stand-fast-night", "custom/tank-stand-fast") }
}

#[test]
fn shared_round_trip() {
    let mut remote = MemoryRemote::default();
    let all = [sf(), child()];
    let r = publish(&mut remote, &all, &all, Some("a.user")).unwrap();
    assert_eq!(r.created, [sf().id, child().id]);
    let elsewhere = pull(&remote, &[]).unwrap();
    assert_eq!(elsewhere.templates, all);
    assert_eq!(elsewhere.added.len(), 2);
    assert_eq!(remote.get(&child().id).unwrap().parent_version, Some(1));
}

#[test]
fn a_newer_shared_version_is_never_overwritten_pulling_takes_it() {
    let mut remote = MemoryRemote::default();
    publish(&mut remote, &[Template { version: 3, ..sf() }], &[sf()], None).unwrap();
    let r = publish(&mut remote, &[sf()], &[sf()], None).unwrap();
    assert_eq!(r.conflicts, vec![Conflict { id: sf().id, mine: 1, theirs: 3, by: None }]);
    let p = pull(&remote, &[sf()]).unwrap();
    assert_eq!(p.updated, [sf().id]);
    assert_eq!(p.templates[0].version, 3);
}

#[test]
fn flags_a_child_built_on_a_parent_version_that_has_moved_on() {
    let mut remote = MemoryRemote::default();
    publish(&mut remote, &[sf(), child()], &[sf(), child()], None).unwrap();
    publish(&mut remote, &[Template { version: 2, ..sf() }], &[sf(), child()], None).unwrap();
    assert_eq!(pull(&remote, &[]).unwrap().stale_parents, [child().id]);
}

#[test]
fn pull_refuses_bodies_for_another_template_and_core_ids() {
    let mut remote = MemoryRemote::default();
    publish(&mut remote, &[sf()], &[sf()], None).unwrap();
    remote.objects[0].body = serde_json::to_string(&child()).unwrap();
    let core = bgws_behaviour::shared::to_shared(&Template::new("core/x", 1), &[], None);
    remote.objects.push(core);
    let p = pull(&remote, &[]).unwrap();
    assert!(p.problems[0].contains("the body is for"));
    assert!(p.problems[1].contains("only custom templates are shared"));
}
