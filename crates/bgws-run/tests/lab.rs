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
