//! Whole-pipeline tests: the store, deception, DF, knowledge, the COP, and
//! the §17 properties (unique stable ids, SPD covariances, order-independent
//! association, truth never in a view).

use crate::*;
use bgws_core::{EntityId, Enu, Figures, HqId, MessageId, Side, SimDuration, SimTime, TrackId};
use nalgebra::Matrix3;
use proptest::prelude::*;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use std::collections::BTreeSet;

fn params() -> KnowParams {
    KnowParams::defaults()
}

fn fix(time: f64, observer: u32, truth: Option<u32>, pos: Enu, sigma: f64) -> Detection {
    Detection {
        time: SimTime::from_secs(time),
        observer: EntityId(observer),
        sensor: SensorKind::Radar,
        domain: Domain::Land,
        motion: MotionHint::Moving,
        measurement: Measurement::Position { pos, cov: Matrix3::from_diagonal_element(sigma * sigma) },
        class: ClassEvidence::detect_only(),
        truth: truth.map(|t| TruthTag::of(EntityId(t))).unwrap_or(TruthTag::none()),
    }
}

#[test]
fn figures_build_and_are_all_used() {
    let f = Figures::from_list(figures()).unwrap();
    let _ = KnowParams::from_figures(&f);
    assert!(f.unused().is_empty(), "unused figures: {:?}", f.unused());
    assert!(f.iter().all(|x| x.key.starts_with("know.")));
}

#[test]
fn one_target_one_confirmed_track() {
    let p = params();
    let mut store = TrackStore::new(Side::Blue);
    let mut rng = ChaCha8Rng::seed_from_u64(11);
    let cov = Matrix3::from_diagonal_element(400.0);
    for k in 0..20 {
        let t = 5.0 * k as f64;
        let truth = Enu::new(1000.0 + 6.0 * t, 2000.0 - 3.0 * t, 0.0);
        let mut d = fix(t, 1, Some(100), truth, 20.0);
        d.measurement = Measurement::noisy_position(truth, cov, &mut rng).unwrap();
        store.ingest(vec![d], &p);
    }
    let views = store.confirmed(&p);
    assert_eq!(views.len(), 1);
    let v = &views[0];
    assert_eq!(v.id, TrackId(1));
    assert!((v.velocity.e - 6.0).abs() < 1.5 && (v.velocity.n + 3.0).abs() < 1.5, "{:?}", v.velocity);
    assert!(v.quality > 8);
    assert_eq!(store.audit().truth_of(v.id), Some(EntityId(100)));
    assert_eq!(v.sources, vec![EntityId(1)]);
}

#[test]
fn duplicate_tracks_arise_and_merge() {
    let p = params();
    let mut store = TrackStore::new(Side::Blue);
    let truth = Enu::new(5000.0, 5000.0, 0.0);
    let mut merged = false;
    for k in 0..4 {
        let t = 10.0 * k as f64;
        // Two observers see the same tank in the same scan, slightly apart.
        let a = fix(t, 1, Some(7), Enu::new(truth.e + 5.0, truth.n, 0.0), 30.0);
        let b = fix(t, 2, Some(7), Enu::new(truth.e - 5.0, truth.n + 3.0, 0.0), 30.0);
        let reports = store.ingest(vec![a, b], &p);
        merged |= reports.iter().any(|r| !r.merges.is_empty());
    }
    assert!(merged, "the duplicate should have been merged");
    let live = store.audit().live_by_truth();
    assert_eq!(live.get(&EntityId(7)).map(Vec::len), Some(1));
    assert_eq!(store.len(), 1);
    assert_eq!(store.views(&p)[0].sources, vec![EntityId(1), EntityId(2)]);
}

#[test]
fn two_separated_targets_stay_two_tracks() {
    let p = params();
    let mut store = TrackStore::new(Side::Red);
    for k in 0..6 {
        let t = 10.0 * k as f64;
        store.ingest(
            vec![
                fix(t, 1, Some(1), Enu::new(0.0, 0.0 + 5.0 * t, 0.0), 10.0),
                fix(t, 1, Some(2), Enu::new(300.0, 0.0 + 5.0 * t, 0.0), 10.0),
            ],
            &p,
        );
    }
    let v = store.confirmed(&p);
    assert_eq!(v.len(), 2);
    let a = store.audit();
    assert_ne!(a.truth_of(v[0].id), a.truth_of(v[1].id));
}

#[test]
fn spoofed_detections_make_a_false_track_the_side_cannot_tell() {
    let p = params();
    let mut store = TrackStore::new(Side::Blue);
    for k in 0..5 {
        let t = 5.0 * k as f64;
        store.ingest(vec![fix(t, 3, None, Enu::new(800.0, 800.0, 0.0), 15.0)], &p);
    }
    let v = store.confirmed(&p);
    assert_eq!(v.len(), 1, "a convincing spoof becomes a confirmed track");
    assert!(store.audit().is_false(v[0].id));
}

#[test]
fn clutter_is_not_confirmed_and_stale_tracks_drop() {
    let p = params();
    let mut store = TrackStore::new(Side::Blue);
    // One clutter plot, then the same radar keeps looking elsewhere.
    store.ingest(vec![fix(0.0, 1, None, Enu::new(0.0, 0.0, 0.0), 10.0)], &p);
    let mut dropped = vec![];
    for k in 1..4 {
        let t = 5.0 * k as f64;
        for r in store.ingest(vec![fix(t, 1, Some(9), Enu::new(20_000.0, 0.0, 0.0), 10.0)], &p) {
            dropped.extend(r.dropped);
        }
    }
    assert!(dropped.contains(&(TrackId(1), DropReason::NotConfirmed)));
    // The real track, unseen for longer than the land coast time, drops.
    let later = SimTime::from_secs(15.0) + p.land.max_coast + SimDuration::from_secs(1.0);
    let gone = store.predict_to(later, &p);
    assert!(gone.iter().any(|(_, why)| matches!(why, DropReason::Stale | DropReason::TooUncertain)));
    assert!(store.is_empty());
    // Ids are never reused.
    store.ingest(vec![fix(later.secs(), 1, Some(9), Enu::new(0.0, 0.0, 0.0), 10.0)], &p);
    assert!(store.ids()[0] > TrackId(2));
}

#[test]
fn decoy_is_a_probable_mbt_until_a_good_look() {
    let p = params();
    let mut store = TrackStore::new(Side::Blue);
    let mut rng = ChaCha8Rng::seed_from_u64(5);
    let at = Enu::new(3000.0, 1000.0, 0.0);
    for k in 0..8 {
        let mut d = fix(10.0 * k as f64, 4, Some(50), at, 20.0);
        d.motion = MotionHint::Static;
        d.sensor = SensorKind::Visual;
        d.class = p.confusion.draw_report(Hypothesis::LandDecoy, IdLevel::Recognise, false, &mut rng);
        store.ingest(vec![d], &p);
    }
    let v = &store.confirmed(&p)[0];
    assert_eq!(v.model, MotionModel::Static);
    assert_eq!(v.class_top[0].0, Hypothesis::Mbt, "{:?}", v.class_top);
    assert!(v.class_top[0].1 > 0.5);
    // A close look through a thermal sight that resolves decoys.
    for k in 0..2 {
        let mut d = fix(100.0 + k as f64, 5, Some(50), at, 10.0);
        d.motion = MotionHint::Static;
        d.sensor = SensorKind::Thermal;
        d.class = ClassEvidence { level: IdLevel::Identify, reported: Some(Hypothesis::LandDecoy), resolves_decoys: true };
        store.ingest(vec![d], &p);
    }
    let v = &store.confirmed(&p)[0];
    assert_eq!(v.class_top[0].0, Hypothesis::LandDecoy, "{:?}", v.class_top);
}

#[test]
fn df_bearings_cross_into_a_track() {
    let p = params();
    let mut store = TrackStore::new(Side::Blue);
    let emitter = Enu::new(12_000.0, 20_000.0, 0.0);
    let mut rng = ChaCha8Rng::seed_from_u64(9);
    let bearing = |obs: u32, origin: Enu, t: f64, rng: &mut ChaCha8Rng| Detection {
        time: SimTime::from_secs(t),
        observer: EntityId(obs),
        sensor: SensorKind::Esm,
        domain: Domain::Land,
        motion: MotionHint::Static,
        measurement: Measurement::noisy_bearing(origin, emitter, 0.01, rng),
        class: ClassEvidence::detect_only(),
        truth: TruthTag::of(EntityId(77)),
    };
    let r = store.ingest(vec![bearing(1, Enu::new(0.0, 0.0, 0.0), 0.0, &mut rng)], &p);
    assert_eq!(r[0].detections[0].outcome, AssocOutcome::DfPending);
    assert!(store.is_empty());
    let r = store.ingest(vec![bearing(2, Enu::new(25_000.0, 0.0, 0.0), 30.0, &mut rng)], &p);
    assert_eq!(r[0].detections[0].outcome, AssocOutcome::DfFix);
    let v = &store.views(&p)[0];
    let err = (v.position.e - emitter.e).hypot(v.position.n - emitter.n);
    assert!(err < 3.0 * v.ellipse.semi_major + 50.0, "DF error {err} vs σ {:?}", v.ellipse);
    // Further bearings update the track (EKF) and shrink it.
    let before = v.ellipse.semi_major;
    store.ingest(vec![bearing(1, Enu::new(0.0, 0.0, 0.0), 60.0, &mut rng)], &p);
    store.ingest(vec![bearing(2, Enu::new(25_000.0, 0.0, 0.0), 61.0, &mut rng)], &p);
    let v = &store.views(&p)[0];
    assert!(v.ellipse.semi_major < before);
    assert_eq!(store.audit().truth_of(v.id), Some(EntityId(77)));
}

#[test]
fn agent_knows_only_what_reached_it() {
    let p = params();
    let mut store = TrackStore::new(Side::Blue);
    let scans = store.ingest(vec![fix(0.0, 1, Some(5), Enu::new(100.0, 100.0, 0.0), 10.0)], &p);
    let mut seer = AgentKnowledge::new(EntityId(1), Side::Blue);
    let mut other = AgentKnowledge::new(EntityId(2), Side::Blue);
    seer.absorb_scan(&scans[0], &store, &p);
    other.absorb_scan(&scans[0], &store, &p);
    assert_eq!(seer.picture(SimTime::from_secs(1.0), &p).len(), 1);
    assert!(other.picture(SimTime::from_secs(1.0), &p).is_empty());
    // A contact report reaches the other unit 15 s later.
    let report = store.view(TrackId(1), &p).unwrap();
    assert!(other.hear(report.clone(), SimTime::from_secs(15.0), Some(EntityId(1))));
    assert!(!other.hear(report, SimTime::from_secs(16.0), None), "not newer: not taken");
    let pic = other.picture(SimTime::from_secs(20.0), &p);
    assert!(matches!(pic[0].origin, KnowledgeOrigin::Told { .. }));
    // Its uncertainty has grown with the 20 s since the sighting.
    assert!(pic[0].ellipse.semi_major > seer.own[&TrackId(1)].ellipse.semi_major);
    other.forget(SimTime::from_secs(500.0), &BTreeSet::new(), &p);
    assert!(other.told.is_empty());
}

#[test]
fn cop_does_not_double_count_a_relayed_report() {
    let p = params();
    let mut store = TrackStore::new(Side::Blue);
    store.ingest(vec![fix(0.0, 1, Some(5), Enu::new(100.0, 100.0, 0.0), 30.0)], &p);
    let r = store.view(TrackId(1), &p).unwrap();
    let mut cop = Cop::new(HqId(1), Side::Blue);
    assert_eq!(cop.receive(&r, EntityId(1), SimTime::from_secs(5.0), &p).outcome, CopOutcome::New);
    let before = cop.entries[&TrackId(1)].fused.estimate.p;
    // The same report arrives again by a second path.
    let u = cop.receive(&r, EntityId(3), SimTime::from_secs(9.0), &p);
    assert_eq!(u.outcome, CopOutcome::Fused);
    let after = cop.entries[&TrackId(1)].fused.estimate.p;
    assert!((after.trace() - before.trace()).abs() < 1e-6 * before.trace(), "CI must not shrink a duplicate");
    // A report of the same object from another store (another id) gates in and fuses.
    let mut other_store = TrackStore::new(Side::Blue);
    other_store.ingest(vec![fix(0.0, 8, Some(5), Enu::new(110.0, 95.0, 0.0), 30.0)], &p);
    let mut r2 = other_store.view(TrackId(1), &p).unwrap();
    r2.id = TrackId(1000);
    let u = cop.receive(&r2, EntityId(8), SimTime::from_secs(10.0), &p);
    assert_eq!(u.outcome, CopOutcome::Fused);
    assert!(u.d2.is_some());
    assert_eq!(cop.entries.len(), 1);
    let fused = cop.entries[&TrackId(1)].fused.estimate.p;
    // CI is conservative: equal-quality reports never claim more than the best
    // of them (the price of being safe against double counting).
    assert!(fused.trace() <= before.trace() * (1.0 + 1e-9));
    let e = cop.entries[&TrackId(1)].fused.position;
    assert!(e.e > 100.0 && e.e < 110.0, "fused position lies between the reports: {e:?}");
    let view = SideView::for_hq(&cop, SimTime::from_secs(12.0), vec![], vec![], &p);
    assert_eq!(view.tracks.len(), 1);
    assert_eq!(cop.prune(SimTime::from_secs(1e5), &p).len(), 1);
}

/// A random scene: targets (some spoofed) seen by several observers over a
/// few scans, with seeded noise.
fn scene(seed: u64, targets: usize, scans: usize) -> Vec<Vec<Detection>> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut out = vec![];
    let starts: Vec<(f64, f64, f64, f64)> = (0..targets)
        .map(|i| {
            use rand::Rng;
            (rng.gen_range(-5000.0..5000.0), rng.gen_range(-5000.0..5000.0), rng.gen_range(-8.0..8.0), (i % 3) as f64)
        })
        .collect();
    for s in 0..scans {
        let t = 5.0 * s as f64;
        let mut scan = vec![];
        for (i, &(e, n, v, obs)) in starts.iter().enumerate() {
            let truth = Enu::new(e + v * t, n + 0.5 * v * t, 0.0);
            let cov = Matrix3::from_diagonal_element(400.0);
            let m = Measurement::noisy_position(truth, cov, &mut rng).unwrap();
            let truth_tag = if i % 5 == 4 { None } else { Some(i as u32 + 100) };
            let mut d = fix(t, obs as u32 + 1, truth_tag, truth, 20.0);
            d.measurement = m;
            scan.push(d);
        }
        out.push(scan);
    }
    out
}

fn assert_no_truth(json: &str) {
    for banned in ["truth", "audit", "contributions", "merged_into", "expired_bearings"] {
        assert!(!json.contains(banned), "`{banned}` leaked into a view: {json}");
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 24, ..ProptestConfig::default() })]

    #[test]
    fn ids_unique_stable_and_covariances_spd(seed in 0u64..10_000, targets in 1usize..8) {
        let p = params();
        let mut store = TrackStore::new(Side::Blue);
        let mut seen: BTreeSet<TrackId> = BTreeSet::new();
        let mut gone: BTreeSet<TrackId> = BTreeSet::new();
        for scan in scene(seed, targets, 6) {
            let before: BTreeSet<TrackId> = store.ids().into_iter().collect();
            let reports = store.ingest(scan, &p);
            let after: BTreeSet<TrackId> = store.ids().into_iter().collect();
            for r in &reports {
                gone.extend(r.dropped.iter().map(|d| d.0));
                gone.extend(r.merges.iter().map(|m| m.absorbed));
            }
            // A track that existed and was not dropped or merged keeps its id.
            for id in &before {
                prop_assert!(after.contains(id) || gone.contains(id));
            }
            // A new id was never used before.
            for id in after.difference(&before) {
                prop_assert!(!seen.contains(id));
            }
            seen.extend(after.iter().copied());
            for id in &after {
                prop_assert!(crate::stats::is_spd(store.covariance(*id).unwrap()));
            }
        }
    }

    #[test]
    fn association_is_order_independent(seed in 0u64..10_000, targets in 1usize..7, rot in 0usize..7) {
        let p = params();
        let mut a = TrackStore::new(Side::Blue);
        let mut b = TrackStore::new(Side::Blue);
        for scan in scene(seed, targets, 5) {
            let mut shuffled = scan.clone();
            shuffled.reverse();
            let k = rot % shuffled.len().max(1);
            shuffled.rotate_left(k);
            a.ingest(scan, &p);
            b.ingest(shuffled, &p);
        }
        let ja = serde_json::to_string(&a.views(&p)).unwrap();
        let jb = serde_json::to_string(&b.views(&p)).unwrap();
        prop_assert_eq!(ja, jb);
    }

    #[test]
    fn truth_never_reaches_a_side_view(seed in 0u64..10_000, targets in 1usize..7) {
        let p = params();
        let mut store = TrackStore::new(Side::Blue);
        let mut agent = AgentKnowledge::new(EntityId(1), Side::Blue);
        let mut cop = Cop::new(HqId(1), Side::Blue);
        let mut last = SimTime::ZERO;
        for scan in scene(seed, targets, 5) {
            for r in store.ingest(scan, &p) {
                agent.absorb_scan(&r, &store, &p);
                last = r.time;
            }
            for v in store.views(&p) {
                cop.receive(&v, EntityId(2), last, &p);
            }
        }
        let own = vec![OwnForceView {
            id: EntityId(1), believed_position: Enu::default(), position_sigma: 30.0, velocity: Enu::default(),
            strength: 1.0, reported_at: last, status: "in position".into(),
        }];
        let msgs = vec![MessageView {
            id: MessageId(1), from: EntityId(2), sent: last, received: last, kind: "contact".into(),
            text: "tanks".into(), tracks: store.ids(),
        }];
        for view in [
            SideView::for_agent(&agent, last, own.clone(), msgs.clone(), &p),
            SideView::for_hq(&cop, last, own, msgs, &p),
        ] {
            let json = serde_json::to_string(&view).unwrap();
            assert_no_truth(&json);
            let back: SideView = serde_json::from_str(&json).unwrap();
            prop_assert_eq!(back.tracks.len(), view.tracks.len());
        }
        assert_no_truth(&serde_json::to_string(&store.views(&p)).unwrap());
        // The audit, by contrast, does know.
        prop_assert!(serde_json::to_string(store.audit()).unwrap().contains("truth"));
    }
}
