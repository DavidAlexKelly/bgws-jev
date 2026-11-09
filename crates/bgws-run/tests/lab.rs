//! The decision lab: moments collected from a game replay to the question
//! they came from, a forced answer changes what happens next, and the
//! report tallies the yardsticks.

use bgws_run::lab::{Lab, Weights};
use bgws_run::{default_figures, load_scenario, DeciderChoice, Host};
use std::path::PathBuf;

fn scenario() -> bgws_scenario::v4::ScenarioV4 {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    load_scenario(&root.join("data/scenarios/skirmish-advance-to-contact-v1.json")).unwrap()
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("bgws-lab-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

#[test]
fn forcing_the_answer_given_replays_the_game_and_another_changes_it() {
    let dir = tmp("force");
    let lab = Lab::open(&dir).unwrap();
    let ids = lab.collect(&scenario(), 1, DeciderChoice::Rules, 40.0, default_figures()).unwrap();
    assert!(ids.len() > 5, "a skirmish has unit decisions: {}", ids.len());
    let m = lab.moment(&ids[ids.len() / 2]).unwrap();
    let record = lab.game(&m.game).unwrap();
    assert!(m.question.state.is_some(), "kept with the unit's state");
    let chosen = m.chosen.clone().unwrap().choice;
    let end = record.ended_at.unwrap();
    let play = |choice: &str, reseed: Option<u64>| {
        let mut h = Host::replay(bgws_scenario::record::GameRecord { ended_at: None, fingerprint: None, outcome: None, ..record.clone() }, default_figures()).unwrap();
        h.force_answer(m.question.id, choice, reseed);
        h.play_until(end, |_| {}).unwrap();
        assert!(h.forced(), "the question came up");
        h.sim.fingerprint()
    };
    // The game was played on the rules: the same answer and the same dice give the same game.
    assert_eq!(play(&chosen, None), record.fingerprint.unwrap(), "a play-out of the answer given, without new dice, is the game itself");
    // New dice give another game.
    assert_ne!(play(&chosen, Some(1)), record.fingerprint.unwrap());
    // Some other option changes the game, at some moment.
    let mut changed = false;
    for id in &ids {
        let m2 = lab.moment(id).unwrap();
        let r2 = lab.game(&m2.game).unwrap();
        let c2 = m2.chosen.clone().unwrap().choice;
        if let Some(other) = m2.question.options.iter().find(|o| o.id != c2) {
            let mut h = Host::replay(bgws_scenario::record::GameRecord { ended_at: None, fingerprint: None, outcome: None, ..r2.clone() }, default_figures()).unwrap();
            h.force_answer(m2.question.id, &other.id, None);
            h.play_until(r2.ended_at.unwrap(), |_| {}).unwrap();
            if h.sim.fingerprint() != r2.fingerprint.unwrap() {
                changed = true;
                break;
            }
        }
    }
    assert!(changed, "forcing another option changes what happens");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn play_outs_score_every_option_and_the_report_counts_them() {
    let dir = tmp("playout");
    let lab = Lab::open(&dir).unwrap();
    let ids = lab.collect(&scenario(), 2, DeciderChoice::Rules, 40.0, default_figures()).unwrap();
    let id = &ids[0];
    let m = lab.playout(id, 3, 600.0, Weights::default(), &default_figures()).unwrap();
    let p = m.playouts.as_ref().unwrap();
    assert_eq!(p.options.len(), m.question.options.len());
    assert!(p.options.iter().all(|o| o.values.len() == 3 && o.mean.is_finite()));
    assert!(p.near_best.contains(&p.best));
    // A label and a graded decider.
    let best = p.best.clone();
    lab.label(id, bgws_run::lab::Label { best: Some(best.clone()), acceptable: vec![], flagged: true, note: "test".into() }).unwrap();
    assert!(lab.label(id, bgws_run::lab::Label { best: Some("nonsense".into()), ..Default::default() }).is_err());
    let (d, name) = bgws_run::lab::decider_for(DeciderChoice::Rules, &default_figures(), None, false, ("", "")).unwrap();
    assert_eq!(lab.grade(d, &name, |m| m.id == *id).unwrap(), 1);
    let r = lab.report().unwrap();
    assert_eq!((r.moments, r.played_out, r.labelled, r.flagged), (ids.len(), 1, 1, 1));
    let row = |who: &str| r.rows.iter().find(|x| x.decider == who && x.point == "all").unwrap();
    // Scored only on decisive moments.
    assert_eq!(row("rules").played_out, usize::from(p.decisive));
    assert_eq!(row("rules (asked)").moments, 1);
    assert!(row("rules (asked)").agrees_with_labels.is_some());
    eprintln!("{}", r.table());
    let _ = std::fs::remove_dir_all(dir);
}

/// A game added before it has been played (or while questions wait for an
/// answer) ends where its record does: the questions handed out at that
/// moment, not yet answered, are not a divergence.
#[test]
fn a_game_added_before_its_questions_are_answered_imports_what_there_is() {
    let dir = tmp("early");
    let lab = Lab::open(&dir).unwrap();
    let sc = scenario();
    let h = Host::new(&sc, sc.seed, default_figures(), &bgws_run::HostOptions { deciders: DeciderChoice::Rules, ..Default::default() }).unwrap();
    let ids = lab.import(h.finished_record(), default_figures()).expect("an unplayed game imports");
    assert!(ids.is_empty(), "nothing decided yet: {ids:?}");
    // Played a while, then added: its decisions so far.
    let mut h = Host::new(&sc, sc.seed, default_figures(), &bgws_run::HostOptions { deciders: DeciderChoice::Rules, ..Default::default() }).unwrap();
    h.play_until(bgws_core::SimTime::from_secs(1_800.0), |_| {}).unwrap();
    let ids = lab.import(h.finished_record(), default_figures()).unwrap();
    assert!(!ids.is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

/// Every shipped decision case builds the way the game builds a question:
/// its options exist after the gates, its expectations name offered
/// options, and its variations build too. The rules decider can be judged
/// against them.
#[test]
fn the_shipped_decision_cases_build_and_can_be_judged() {
    use bgws_run::cases;
    let dir = tmp("cases");
    let (bases, list) = cases::load(&cases::case_dirs(&dir)).unwrap();
    assert!(list.len() >= 15, "a starter set: {}", list.len());
    let points: std::collections::BTreeSet<&str> = list.iter().map(|c| c.point.as_str()).collect();
    assert!(points.len() >= 12, "every decision point but a few: {points:?}");
    let mut b = cases::Builder::new(bases, &default_figures()).unwrap();
    let items = cases::build_all(&mut b, &list).unwrap();
    // Each question carries the case's state as fields, and its words.
    assert!(items.iter().all(|i| i.question.state.is_some() && !i.question.situation.is_empty()));
    // A case whose enemy closes: the derived fields follow the range.
    let far = items.iter().find(|i| i.variant.ends_with("4.5 km, out of reach")).unwrap();
    assert_eq!(far.question.state.as_ref().unwrap()["enemiesSeen"][0]["inOurReach"], false);
    let fb = cases::judge(&list, &items, &cases::fallback_answers(&items), "template fallback");
    eprintln!("{}", fb.table());
    assert_eq!(fb.invariant, fb.invariance_checks, "the fallback does not care about order or names");
    // The rules decider, asked as Jev would be.
    let (d, name) = cases::decider(DeciderChoice::Rules, &default_figures(), None, false).unwrap();
    let run = cases::run(&list, &items, d, &name).unwrap();
    assert_eq!(run.items.len(), items.len());
    // A case from a lab moment.
    let lab = Lab::open(&dir).unwrap();
    let ids = lab.collect(&scenario(), 1, DeciderChoice::Rules, 40.0, default_figures()).unwrap();
    let m = lab.label(&ids[0], bgws_run::lab::Label { best: Some(lab.moment(&ids[0]).unwrap().question.options[0].id.clone()), ..Default::default() }).unwrap();
    let c = cases::case_from_moment(&m, "test").unwrap();
    assert_eq!(b.items(&c).unwrap()[0].acceptable, c.acceptable);
    let _ = std::fs::remove_dir_all(dir);
}

/// The engagement suite pairs each game with a rules baseline on the same
/// seed: with the rules themselves under test, the two games are the same,
/// so every difference is exactly zero. Its worst games can be kept in the lab.
#[test]
fn the_suite_pairs_games_with_the_rules_baseline() {
    use bgws_run::suite;
    let entries = vec![suite::Entry { scenario: "skirmish-catalogue-armour-v1".into(), minutes: Some(30.0), note: String::new(), held_out: false }];
    let (d, name) = bgws_run::lab::decider_for(DeciderChoice::Rules, &default_figures(), None, false, ("", "")).unwrap();
    let r = suite::run(&entries, &[1, 2], d, &name, &default_figures(), 2, &suite::Options::default(), &|_| {}).unwrap();
    assert_eq!(r.pairs.len(), 4, "two seeds, each side");
    for p in &r.pairs {
        assert_eq!((p.diff.result, p.diff.exchange, p.diff.own_loss, p.diff.survival), (0.0, 0.0, 0.0, 0.0), "{p:?}");
    }
    let all = r.summaries.iter().find(|s| s.scope == "all").unwrap();
    assert_eq!((all.pairs, all.better, all.worse), (4, 0, 0));
    assert!(all.baseline.survival > 0.0 && all.baseline.survival <= 1.0);
    assert!(suite::load_entries().unwrap().len() >= 5, "the shipped suite");
}

/// The tuner's guardrails, without calling anyone: a rewrite that drops a
/// placeholder, names something from the cases or quotes a range is
/// refused; a general one passes; the held-out split is fixed; the
/// confidence cut-off is chosen from the answers; a diff says what changed.
#[test]
fn the_tuners_guardrails_hold() {
    use bgws_decide::jev::Wording;
    use bgws_run::{cases, tune};
    let dir = tmp("tune");
    let (bases, list) = cases::load(&cases::case_dirs(&dir)).unwrap();
    let (train, held) = tune::split(&list, 0.3);
    assert_eq!(train.len() + held.len(), list.len());
    assert!(!held.is_empty() && !train.is_empty());
    assert_eq!(tune::split(&list, 0.3).1.iter().map(|c| &c.id).collect::<Vec<_>>(), held.iter().map(|c| &c.id).collect::<Vec<_>>(), "the split does not move");
    let mut b = cases::Builder::new(bases, &default_figures()).unwrap();
    let items = cases::build_all(&mut b, &list).unwrap();
    let words: std::collections::BTreeSet<String> = ["blue 1 troop".to_string(), "t-72b3".to_string()].into_iter().collect();
    let actions: std::collections::BTreeSet<String> = bgws_behaviour::Vocabulary::builtin().actions.keys().cloned().collect();
    let ok = Wording { balance: "Carry out the intent; stay alive to do it.".into(), ..Default::default() };
    assert!(tune::check(&ok, &words, &actions).is_ok());
    let mut lost = ok.clone();
    lost.land = lost.land.replace("{situation}", "");
    assert!(tune::check(&lost, &words, &actions).unwrap_err().contains("{situation}"));
    let leak = Wording { balance: "If a T-72B3 is near, engage.".into(), ..Default::default() };
    assert!(tune::check(&leak, &words, &actions).unwrap_err().contains("names something"));
    let range = Wording { balance: "Engage inside 1500 m.".into(), ..Default::default() };
    assert!(tune::check(&range, &words, &actions).unwrap_err().contains("three or more digits"));
    let mut unknown = ok.clone();
    unknown.option_words.insert("land.teleport".into(), "Go elsewhere.".into());
    assert!(tune::check(&unknown, &words, &actions).is_err());
    let mut words_ok = ok.clone();
    words_ok.option_words.insert("land.betterPosition".into(), "Move to a firing position with more cover: it is exposed while it moves.".into());
    assert!(tune::check(&words_ok, &words, &actions).is_ok());
    // Option words reach Jev's request; a per-point cut-off is read.
    let q = items.iter().find(|i| i.question.options.iter().any(|o| o.action == "land.betterPosition")).unwrap().question.clone();
    let (body, _) = bgws_decide::jev::build_request_worded("m", q.side, std::slice::from_ref(&q), None, 255, false, &words_ok);
    assert!(body.to_string().contains("exposed while it moves"));
    let mut t = Wording::default();
    t.point_confidence.insert("D1".into(), 0.1);
    assert_eq!((t.threshold("D1", 0.25), t.threshold("D2", 0.25)), (0.1, 0.25));
    // The cut-off from a run's answers: a confident wrong pick and an unsure right one.
    let fb = cases::judge(&list, &items, &cases::fallback_answers(&items), "fallback");
    let th = cases::best_threshold(&fb, 0.25);
    assert!(th.1 >= th.2, "never worse than the old cut-off");
    assert!(tune::diff(&Wording::default(), &words_ok).contains("optionWords.land.betterPosition"));
    assert_eq!(tune::diff(&Wording::default(), &Wording::default()), "(no change)\n");
    let _ = std::fs::remove_dir_all(dir);
}
