//! Questions, the compatibility table, safeguards, the bench and patches.
//! The world-question expectations follow `world/jevWorldDecider.test.ts`
//! and `world/questions.ts`: one choice per agent with doctrine, each option's
//! bias, and the fallback the rules take.

use bgws_behaviour::bench::{bench, compare, BenchStatus};
use bgws_behaviour::facts::NoFacts;
use bgws_behaviour::library::Capability;
use bgws_behaviour::patch::{apply_patch, describe_changes, Patch};
use bgws_behaviour::question::{build_question, highest_bias, needs_asking, Expansion, QuestionContext, QuestionLimits, RulesChoice};
use bgws_behaviour::safeguards::{check_safeguards, is_contact_trigger, SafeguardIssue};
use bgws_behaviour::types::{DropReason, GateName, Template, WalkResult};
use bgws_behaviour::{Emcon, Gates, LoadMode, Registry, Roe, Vocabulary};
use bgws_core::{QuestionId, QuestionKind, SimDuration, SimTime, Side};
use serde_json::json;

fn ctx(trigger: &str, template: &str) -> QuestionContext {
    QuestionContext {
        id: QuestionId(7),
        at: SimTime::from_secs_i(60),
        kind: QuestionKind::Entity,
        agent: "b1".into(),
        side: Side::Blue,
        trigger: trigger.into(),
        template: template.into(),
        situation: "A call for fire has come in from 1 Coy.".into(),
        reaction: SimDuration::from_secs_i(30),
        summaries: Default::default(),
        expansions: Default::default(),
        rule: RulesChoice::HighestBias,
    }
}
const LIMITS: QuestionLimits = QuestionLimits { max_options: 16 };

#[test]
fn a_world_question_carries_doctrine_biases_and_the_rules_choice() {
    let reg = Registry::builtin().unwrap();
    let (t, walk) = reg.decide(&["core/mortar"], "fireRequestReceived", &NoFacts, &Gates::open()).unwrap().unwrap();
    assert_eq!(t.lineage, ["core/mortar", "core/default", "core/world"]);
    let q = build_question(&walk, &ctx("fireRequestReceived", &t.id), reg.vocabulary(), LIMITS).unwrap();
    assert_eq!(q.title, "Fire request");
    assert_eq!(q.node, "w.fireRequestReceived");
    assert_eq!(q.doctrine.len(), 2, "core/world's doctrine, then the mortar's");
    assert_eq!(q.options.iter().map(|o| (o.id.as_str(), o.bias)).collect::<Vec<_>>(), [("fireNow", 1), ("fireWhenLaid", 0), ("fireThenScoot", 0), ("refuse", 0)]);
    assert_eq!(q.options[0].summary, "Fire the mission now");
    assert_eq!(q.fallback, "fireNow");
    assert_eq!(q.rules_choice, "fireNow");
    assert_eq!(q.reaction, SimDuration::from_secs_i(30));
    assert!(needs_asking(&q));
}

#[test]
fn rules_choice_is_highest_bias_then_first_or_the_templates_rule() {
    let reg = Registry::builtin().unwrap();
    let (t, walk) = reg.decide(&["core/artillery"], "counterBatteryThreat", &NoFacts, &Gates::open()).unwrap().unwrap();
    let mut c = ctx("counterBatteryThreat", &t.id);
    let q = build_question(&walk, &c, reg.vocabulary(), LIMITS).unwrap();
    // artillery biases displaceNow +1: highest bias wins over the fallback.
    assert_eq!(q.fallback, "finishThenDisplace");
    assert_eq!(q.rules_choice, "displaceNow");
    c.rule = RulesChoice::Fallback;
    assert_eq!(build_question(&walk, &c, reg.vocabulary(), LIMITS).unwrap().rules_choice, "finishThenDisplace");
    c.rule = RulesChoice::Option("stay".into());
    assert_eq!(build_question(&walk, &c, reg.vocabulary(), LIMITS).unwrap().rules_choice, "stay");
    c.rule = RulesChoice::Option("teleport".into());
    assert_eq!(build_question(&walk, &c, reg.vocabulary(), LIMITS).unwrap().rules_choice, "displaceNow");
}

#[test]
fn expansions_summaries_and_drills() {
    let reg = Registry::builtin().unwrap();
    let (t, walk) = reg.decide(&["core/drone"], "targetFound", &NoFacts, &Gates::open()).unwrap().unwrap();
    let mut c = ctx("targetFound", &t.id);
    c.summaries.insert("report".into(), "report the T-72 at 2 km and keep watching".into());
    c.expansions.insert("strike".into(), vec![Expansion { key: "r7".into(), summary: "strike R7".into(), params: [("target".to_string(), json!("r7"))].into_iter().collect() }]);
    c.expansions.insert("handOff".into(), vec![]);
    let q = build_question(&walk, &c, reg.vocabulary(), LIMITS).unwrap();
    assert_eq!(q.options.iter().map(|o| o.id.as_str()).collect::<Vec<_>>(), ["report", "callFire", "strike:r7"]);
    assert_eq!(q.options[0].summary, "report the T-72 at 2 km and keep watching");
    assert_eq!(q.options[2].params.get("target"), Some(&json!("r7")));

    // A drill: one option named after its node, asked of no one.
    let drill = WalkResult::Do { node_id: "smoke".into(), action: "land.goQuietInCover".into(), params: Default::default(), path: vec!["smoke".into()] };
    let q = build_question(&drill, &ctx("underFire", "custom/x"), reg.vocabulary(), LIMITS).unwrap();
    assert_eq!((q.title.as_str(), q.fallback.as_str(), q.options.len()), ("underFire (drill)", "smoke", 1));
    assert!(!needs_asking(&q));
    assert!(build_question(&WalkResult::None { path: vec![], blocked: None }, &ctx("x", "y"), reg.vocabulary(), LIMITS).is_none());
}

#[test]
fn option_lists_are_bounded_keeping_fallback_and_rules_choice() {
    let reg = Registry::builtin().unwrap();
    let (t, walk) = reg.decide(&["core/default"], "D1", &NoFacts, &Gates::open()).unwrap().unwrap();
    let mut c = ctx("D1", &t.id);
    c.rule = RulesChoice::Option("pullBack".into());
    let q = build_question(&walk, &c, reg.vocabulary(), QuestionLimits { max_options: 3 }).unwrap();
    assert_eq!(q.options.iter().map(|o| o.id.as_str()).collect::<Vec<_>>(), ["keep", "engage", "pullBack"]);
    assert_eq!(highest_bias(&q.options), "keep");
}

#[test]
fn compatible_loading_reports_unknown_actions_and_degrades_to_the_fallback() {
    // An engine that lacks pulling back and assaulting.
    let vocab = Vocabulary::builtin();
    let supported: Vec<String> = vocab.actions.keys().filter(|a| !matches!(a.as_str(), "land.pullBackToFriends" | "land.assault")).cloned().collect();
    let vocab = vocab.restrict_actions(supported.iter().map(String::as_str));
    assert!(Registry::with_custom(&[], &vocab, LoadMode::Strict).is_err());
    let reg = Registry::with_custom(&[], &vocab, LoadMode::Compatible).unwrap();
    assert_eq!(reg.compatibility().unsupported(), ["land.assault", "land.pullBackToFriends"]);
    assert!(reg.compatibility().markdown().contains("| `land.assault` | **no**: falls back | core/default.nodes.D2.options[4] |"));
    let w = reg.walk(reg.resolved("core/default").unwrap(), "D1", &NoFacts, &Gates::open()).unwrap();
    let WalkResult::Choose { dropped, .. } = &w else { panic!() };
    assert_eq!(dropped[0].why, DropReason::Unsupported);
    assert!(!w.option_ids().contains(&"pullBack"));
}

#[test]
fn a_template_naming_an_unknown_fact_fails_even_when_compatible() {
    let bad = Template {
        when: Some([("engage".to_string(), bgws_behaviour::Condition::fact("enemy.morale", bgws_behaviour::types::Comparison::Lt, 3.0))].into_iter().collect()),
        ..Template::variant("custom/bad", "core/default")
    };
    let e = Registry::with_custom(&[bad], &Vocabulary::builtin(), LoadMode::Compatible).unwrap_err();
    assert!(e.problems.iter().any(|p| p.kind == bgws_behaviour::schema::ProblemKind::UnknownFact { fact: "enemy.morale".into() }));
}

#[test]
fn safeguards_on_the_shipped_templates() {
    let reg = Registry::builtin().unwrap();
    let warnings = check_safeguards(&reg, 16);
    assert!(warnings.iter().all(|w| w.issue == SafeguardIssue::NoDeescalation), "nothing too long: {warnings:?}");
    // Every land contact decision offers a way to step back.
    assert!(!warnings.iter().any(|w| w.trigger.starts_with('D')), "{warnings:?}");
    // No shipped contact decision, land or world, lacks a way to step back.
    let nodes: std::collections::BTreeSet<_> = warnings.iter().map(|w| w.node.as_str()).collect();
    assert_eq!(nodes.into_iter().collect::<Vec<_>>(), Vec::<&str>::new(), "{warnings:?}");
    // A variant that takes away every way out is flagged.
    let reckless = Template { remove: Some(vec!["pullBack".into(), "quiet".into()]), ..Template::variant("custom/reckless", "core/default") };
    let reg = Registry::with_custom(&[reckless], &Vocabulary::builtin(), LoadMode::Strict).unwrap();
    let w = check_safeguards(&reg, 4);
    assert!(w.iter().any(|w| w.template == "custom/reckless" && w.trigger == "D2" && w.issue == SafeguardIssue::NoDeescalation));
    assert!(w.iter().any(|w| matches!(w.issue, SafeguardIssue::TooManyOptions { count: 6, max: 4 })));
    assert!(is_contact_trigger("underFire") && is_contact_trigger("enemyContactReport") && !is_contact_trigger("bingoFuel"));
}

#[test]
fn the_bench_shows_offered_and_why_not_and_compares_two_templates() {
    let sf = Template { remove: Some(vec!["pullBack".into()]), ..Template::variant("custom/stand-fast", "core/armour") };
    let reg = Registry::with_custom(&[sf], &Vocabulary::builtin(), LoadMode::Strict).unwrap();
    let gates = Gates { roe: Roe::Never, ..Gates::open() };
    let a = bench(&reg, "core/armour", "D1", &NoFacts, &Gates::open()).unwrap();
    let b = bench(&reg, "custom/stand-fast", "D1", &NoFacts, &gates).unwrap();
    assert_eq!(b.lineage, ["custom/stand-fast", "core/armour", "core/default"]);
    assert_eq!(b.offered, ["keep", "observe"]);
    assert!(b.rows.iter().any(|r| r.option == "pullBack" && r.status == BenchStatus::Dropped(DropReason::Removed)));
    assert!(b.rows.iter().any(|r| r.option == "engage" && r.status == BenchStatus::Dropped(DropReason::Gate(GateName::Roe))));
    assert_eq!(a.rules_choice.as_deref(), Some("keep"));
    let d = compare(&a, &b);
    assert_eq!(d.only_in_a, ["engage", "wait", "better", "pullBack"]);
    assert!(d.only_in_b.is_empty());
    // A world trigger on a land template goes through core/world.
    let w = bench(&reg, "core/armour", "minefieldStruck", &NoFacts, &Gates { emcon: Emcon::Silent, capabilities: Some([Capability::Mobility].into_iter().collect()), ..Gates::open() }).unwrap();
    assert_eq!(w.offered, ["pushThroughLane", "reverseOut"]);
    assert_eq!(w.fallback.as_deref(), Some("reverseOut"));
    assert!(bench(&reg, "core/nope", "D1", &NoFacts, &Gates::open()).is_err());
}

#[test]
fn patches_edit_a_variant_without_touching_its_parent() {
    let reg = Registry::builtin().unwrap();
    let base = Template::variant("custom/v", "core/default");
    let resolved = reg.resolved("core/default").unwrap();
    let mut t = base.clone();
    for p in [
        Patch::Remove { option: "pullBack".into() },
        Patch::SetBias { option: "observe".into(), bias: Some(2) },
        Patch::SetDoctrine { text: Some("Hold.".into()) },
        Patch::OverrideNode { node: "D1".into(), title: "Contact!".into(), question: "Now what?".into(), fallback: "observe".into() },
    ] {
        t = apply_patch(&t, resolved, &p).unwrap();
    }
    let reg2 = Registry::with_custom(std::slice::from_ref(&t), &Vocabulary::builtin(), LoadMode::Strict).unwrap();
    let w = reg2.walk(reg2.resolved("custom/v").unwrap(), "D1", &NoFacts, &Gates::open()).unwrap();
    let WalkResult::Choose { title, fallback, options, doctrine, .. } = &w else { panic!() };
    assert_eq!((title.as_str(), fallback.as_deref()), ("Contact!", Some("observe")));
    assert_eq!(options.iter().find(|o| o.id == "observe").unwrap().bias, 2);
    assert_eq!(doctrine, &["Hold.".to_string()]);
    assert!(!w.option_ids().contains(&"pullBack"));
    assert_eq!(describe_changes(&t).len(), 5);
    // Undo everything: back to the bare variant.
    for p in [
        Patch::Restore { option: "pullBack".into() },
        Patch::SetBias { option: "observe".into(), bias: None },
        Patch::SetDoctrine { text: None },
        Patch::ClearNodeOverride { node: "D1".into() },
    ] {
        t = apply_patch(&t, resolved, &p).unwrap();
    }
    assert_eq!(t, base);
    assert!(apply_patch(&t, resolved, &Patch::OverrideNode { node: "D1".into(), title: "x".into(), question: "y".into(), fallback: "nope".into() }).is_err());
}
