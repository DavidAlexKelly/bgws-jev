//! Acceptance checks for the catalogue (§14.1): the shipped data loads,
//! resolves and validates; every TS archetype has a record; class defaults
//! fill a record written with a few numbers; figures override them; and
//! validation catches broken references, bands and ranges. Loaders never
//! panic on bad input.

use bgws_catalogue::*;
use bgws_core::Figures;
use proptest::prelude::*;

fn builtin() -> Catalogue {
    match Catalogue::builtin() {
        Ok(c) => c,
        Err(e) => panic!("builtin catalogue failed to load: {e}"),
    }
}

fn sources_plus(extra: &str) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = BUILTIN_FILES.iter().map(|(n, t)| (n.to_string(), t.to_string())).collect();
    v.push(("zz_extra.json".into(), extra.into()));
    v
}

#[test]
fn shipped_catalogue_loads_and_validates() {
    let c = builtin();
    let problems = c.validate();
    assert!(problems.is_empty(), "{} problems:\n{}", problems.len(), problems.iter().map(|p| p.to_string()).collect::<Vec<_>>().join("\n"));
    assert!(c.platforms().count() >= 100, "{}", c.platforms().count());
    assert!(c.munitions().count() >= 40);
}

#[test]
fn figures_register_cleanly() {
    let figs = figures();
    let f = Figures::from_list(figs.clone()).expect("no duplicates, all in range");
    assert!(f.len() > 300, "{}", f.len());
    assert!(f.figure("catalogue.class.mbt.mover.burnMovingLph").is_some());
    assert_eq!(f.figure("catalogue.class.mbt.mover.burnMovingLph").unwrap().source, "P17, P27");
    assert!(f.figure("catalogue.derive.radar_standard_target_m2").is_some());
}

#[test]
fn every_ts_archetype_and_force_list_key_has_a_record() {
    let c = builtin();
    for a in TS_ARCHETYPES.iter().chain(TS_PLATFORM_KEYS) {
        assert!(c.has(a), "{a}");
    }
    assert_eq!(c.archetype_of("var_11_default"), Some("generic/mbt"));
    assert_eq!(c.archetype_of("var_27_default"), Some("generic/ifv"));
    assert_eq!(c.archetype_of("sec_uk_javelin"), Some("generic/atgmTeam"));
    assert_eq!(c.archetype_of("generic/frigate"), Some("generic/frigate"));
    assert_eq!(archetype_for(&c, "var_11_default", Some(3.0)), "generic/cp-company");
    assert_eq!(archetype_for(&c, "nonsense", None), "generic/infantrySection");
}

#[test]
fn records_carry_the_new_physics() {
    let c = builtin();
    let mbt = c.platform("generic/mbt").unwrap();
    assert_eq!(mbt.mover.kind, MoverKind::Tracked);
    assert!(mbt.mover.cross_country_kmh > 0.0 && mbt.mover.cross_country_kmh < mbt.mover.road_kmh);
    assert!(mbt.body.armour.ke.side < mbt.body.armour.ke.front);
    assert!(mbt.body.armour.ce.front > mbt.body.armour.ke.front, "composite front is better against CE");
    assert!(mbt.signatures.ir.delta_k.moved > mbt.signatures.ir.delta_k.cold);
    assert!(mbt.signatures.rcs_m2("x", Aspect::Side) > mbt.signatures.rcs_m2("x", Aspect::Front));
    assert!(mbt.mover.endurance_h > 5.0, "{}", mbt.mover.endurance_h);
    let gun = c.munition_of("generic/mbt", "tankGun").unwrap();
    assert!(gun.penetration_at(1000.0) > gun.penetration_at(3000.0));
    // Sourced Challenger 2: its own armour by aspect and its own round.
    let ch2 = c.platform("var_11_default").unwrap();
    assert_eq!(ch2.body.armour.ke.front, 700.0);
    assert_eq!(ch2.body.armour.ce.side, 400.0);
    assert!(ch2.body.era);
    assert_eq!(c.munition_of("var_11_default", "tankGun").unwrap().id, "charm3");
    // Inherited from its archetype: fuel burn, crew, sensors.
    assert_eq!(ch2.mover.burn_moving_lph, mbt.mover.burn_moving_lph);
    assert_eq!(ch2.sensors.len(), mbt.sensors.len());
    // A destroyer with VLS cells; a frigate with a towed array.
    let ddg = c.platform("generic/destroyer").unwrap();
    let vls = ddg.mount("vls").unwrap();
    assert_eq!(vls.kind, WeaponKind::Vls);
    assert!(vls.munitions.contains(&"samLong".to_string()));
    let ffg = c.platform("generic/frigate").unwrap();
    assert!(ffg.sensors.iter().any(|s| s.sonar.is_some_and(|x| x.towed)));
    // Submarines are quiet slow and loud fast.
    let ssk = c.platform("generic/ssk").unwrap();
    assert!(ssk.signatures.acoustic.water_db_at(5.0).unwrap() < ssk.signatures.acoustic.water_db_at(30.0).unwrap());
    // The jammer's GNSS radius [P16].
    let jam = c.platform("generic/ewJammer").unwrap();
    assert_eq!(jam.emitters.iter().find(|e| e.kind == EmitterKind::Jammer).unwrap().jam_radii_m["gnss"], 25_000.0);
    // Decoys are flagged.
    assert!(c.platform("generic/owaDecoy").unwrap().signatures.decoy);
}

#[test]
fn derived_radar_power_reproduces_the_quoted_range() {
    let c = builtin();
    for p in c.platforms() {
        for s in &p.sensors {
            if let Some(r) = s.radar {
                let range = radar_range_for(r.pt_w, 5.0, r.gain_db, r.freq_hz, r.bandwidth_hz, r.noise_figure_db, r.losses_db, r.snr_required_db, r.pulses_integrated);
                assert!((range / s.range_max_m - 1.0).abs() < 1e-6, "{} {}: {range} vs {}", p.id, s.id, s.range_max_m);
                assert!(c.band(&s.band).unwrap().contains(r.freq_hz));
            }
        }
    }
}

#[test]
fn every_kind_of_platform_the_brief_lists_is_there() {
    let c = builtin();
    let need = [
        "generic/mbt", "generic/ifv", "generic/infantrySection", "generic/recce", "generic/atgmTeam", "generic/mortar120", "generic/sph155", "generic/mlrs",
        "generic/engineerSection", "generic/cp-brigade", "generic/convoyTruck", "generic/fpvRf", "generic/fpvFibre", "generic/bomberDrone", "generic/quadIsr",
        "generic/fixedWingIsr", "generic/loiter", "generic/owaDrone", "generic/interceptorDrone", "generic/counterDrone", "generic/ewJammer", "generic/esmTeam",
        "generic/shorad", "generic/samMedium", "generic/samLong", "generic/fighter4", "generic/bomber", "generic/strikeFighter", "generic/attackHelo",
        "generic/aew", "generic/tanker", "generic/frigate", "generic/destroyer", "generic/ssk", "generic/usv", "generic/coastalAsm", "generic/satEo",
        "generic/satSar", "generic/satcomGeo", "generic/satcomLeo", "generic/gnss", "generic/cyberCell", "generic/sensorNode", "generic/srbmBattery",
    ];
    for id in need {
        assert!(c.has(id), "{id}");
    }
    for d in [Domain::Land, Domain::Air, Domain::Surface, Domain::Subsurface, Domain::Space, Domain::Node] {
        assert!(!c.platforms_in(d).is_empty(), "{d:?}");
    }
}

#[test]
fn a_record_with_a_few_numbers_is_filled_from_its_class() {
    let extra = r#"{"platforms": [{"id": "test/newTank", "name": "New tank", "class": "mbt", "role": "test", "sidc": "SFGPUCA",
        "mover": {"maxKmh": 70, "fuelCapacityL": 1500},
        "body": {"armour": {"ke": {"front": 800}}},
        "costUsd": 1e7, "rationale": "a test"}]}"#;
    let c = Catalogue::from_sources(&sources_plus(extra)).expect("loads");
    let p = c.platform("test/newTank").unwrap();
    assert_eq!(p.domain, Domain::Land);
    assert_eq!(p.template, "core/armour");
    assert_eq!(p.body.count, 4);
    assert_eq!(p.mover.burn_moving_lph, 227.0);
    assert_eq!(p.mover.road_kmh, 60.0);
    assert!((p.body.armour.ke.side - 240.0).abs() < 1e-9);
    assert_eq!(p.sidc.red, "SHGPUCA--------");
    assert!(!p.sensors.is_empty() && p.sensors.iter().all(|s| s.optics.is_some()));
    assert!(platform_ok(&c, "test/newTank"));
}

fn platform_ok(c: &Catalogue, id: &str) -> bool {
    !c.validate().iter().any(|e| e.to_string().contains(id))
}

#[test]
fn class_defaults_are_figures_a_run_can_override() {
    let mut f = Figures::from_list(figures()).unwrap();
    f.set("catalogue.class.mbt.mover.burnMovingLph", 300.0).unwrap();
    f.set("catalogue.derive.armour_side_of_front", 0.5).unwrap();
    let c = Catalogue::builtin_with(&f).unwrap();
    let mbt = c.platform("generic/mbt").unwrap();
    // generic/mbt states its own burn (from the TS record), so the class figure does not reach it…
    assert_eq!(mbt.mover.burn_moving_lph, 227.0);
    // …but a record that leaves it to the class does.
    assert_eq!(c.platform("generic/mbt").unwrap().body.armour.ke.side, 650.0 * 0.5);
    let extra = r#"{"platforms": [{"id": "test/t", "name": "t", "class": "mbt", "role": "t", "sidc": "SFGPUCA", "costUsd": 1, "rationale": "t"}]}"#;
    let mut list = figures();
    list.retain(|x| x.key != "catalogue.class.mbt.mover.burnMovingLph");
    list.push(bgws_core::figure!("catalogue.class.mbt.mover.burnMovingLph", 300.0, "L/h", 0.0, 50000.0, "default", Default, "test"));
    let f2 = Figures::from_list(list).unwrap();
    let c2 = Catalogue::from_sources_with(&sources_plus(extra), &f2).unwrap();
    assert_eq!(c2.platform("test/t").unwrap().mover.burn_moving_lph, 300.0);
}

#[test]
fn validation_catches_broken_records() {
    let extra = r#"{"platforms": [{"id": "test/bad", "name": "bad", "class": "mbt", "role": "t", "sidc": "SFGPUCA", "costUsd": 1, "rationale": "t",
        "mounts": [{"id": "g", "kind": "directFire", "munitions": ["noSuchRound"], "salvo": 1, "magazine": 1, "reloadS": 1}],
        "radios": [{"bearer": "vhf", "band": "zz"}],
        "mover": {"ceilingM": -5, "cruiseKmh": 500},
        "carried": {"drones": {"archetype": "generic/ghost", "count": 1}},
        "stores": {"ammo": {"nothing": 4}}}]}"#;
    let c = Catalogue::from_sources(&sources_plus(extra)).expect("loads; problems come from validate");
    let errs = c.validate();
    let has = |f: &dyn Fn(&CatalogueError) -> bool| errs.iter().any(f);
    assert!(has(&|e| matches!(e, CatalogueError::UnknownMunition { munition, .. } if munition == "noSuchRound")));
    assert!(has(&|e| matches!(e, CatalogueError::UnknownBand { band, .. } if band == "zz")));
    assert!(has(&|e| matches!(e, CatalogueError::OutOfRange { field, .. } if field == "mover.ceilingM")));
    assert!(has(&|e| matches!(e, CatalogueError::Inconsistent { message, .. } if message.contains("cruise speed"))));
    assert!(has(&|e| matches!(e, CatalogueError::UnknownCarried { .. })));
    assert!(has(&|e| matches!(e, CatalogueError::UnknownAmmo { .. })));
}

#[test]
fn load_errors_are_typed_and_complete() {
    let files = vec![
        ("a.json".to_string(), "{ not json".to_string()),
        ("b.json".to_string(), r#"{"platforms": [{"id": "x", "class": "nope"}, {"id": "y", "class": "mbt", "parent": "missing"}], "weird": 1}"#.to_string()),
        ("c.json".to_string(), r#"{"platforms": [{"id": "p", "parent": "q"}, {"id": "q", "parent": "p"}]}"#.to_string()),
    ];
    let errs = Catalogue::from_sources(&files).unwrap_err().0;
    assert!(errs.iter().any(|e| matches!(e, CatalogueError::Parse { .. })));
    assert!(errs.iter().any(|e| matches!(e, CatalogueError::Layout { .. })));
    assert!(errs.iter().any(|e| matches!(e, CatalogueError::UnknownClass { .. })));
    assert!(errs.iter().any(|e| matches!(e, CatalogueError::UnknownParent { .. })));
    assert!(errs.iter().any(|e| matches!(e, CatalogueError::Cycle { .. })));
    // An empty catalogue is missing every TS archetype.
    let empty = Catalogue::from_sources(&[]).unwrap();
    assert!(empty.validate().iter().filter(|e| matches!(e, CatalogueError::MissingArchetype { .. })).count() >= TS_ARCHETYPES.len());
}

#[test]
fn load_dir_matches_builtin() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/catalogue");
    let a = Catalogue::load_dir(&dir).expect("data dir loads");
    let b = builtin();
    assert_eq!(a.fingerprint(), b.fingerprint());
    assert!(Catalogue::load_dir(std::path::Path::new("/definitely/not/here")).is_err());
}

#[test]
fn resolved_records_round_trip_through_json() {
    let c = builtin();
    for p in c.platforms() {
        let s = serde_json::to_string(p).unwrap();
        let back: Platform = serde_json::from_str(&s).unwrap();
        assert_eq!(&back, p);
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, .. ProptestConfig::default() })]

    #[test]
    fn loader_never_panics_on_random_bytes(bytes in proptest::collection::vec(any::<u8>(), 0..400)) {
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let _ = Catalogue::from_sources(&[("r.json".into(), text)]);
    }

    #[test]
    fn loader_never_panics_on_random_records(
        class in prop_oneof![Just("mbt"), Just("ship"), Just("nope"), Just("satellite")],
        speed in prop_oneof![Just(serde_json::json!(-1)), Just(serde_json::json!("fast")), Just(serde_json::json!(1e308)), Just(serde_json::json!(40))],
        count in prop_oneof![Just(serde_json::json!(-3)), Just(serde_json::json!(2.5)), Just(serde_json::json!(4)), Just(serde_json::Value::Null)],
        sensors in prop_oneof![Just(serde_json::json!([{"kind": "radar"}])), Just(serde_json::json!("x")), Just(serde_json::json!([{"kind": "warp"}]))],
    ) {
        let rec = serde_json::json!({"platforms": [{"id": "t/x", "name": "x", "class": class, "role": "r", "sidc": "S", "costUsd": 1, "rationale": "r",
            "mover": {"maxKmh": speed}, "body": {"count": count}, "sensors": sensors}]});
        if let Ok(c) = Catalogue::from_sources(&sources_plus(&rec.to_string())) {
            let _ = c.validate();
        }
    }
}

/// The importer's output (`imported.json`, generated from the asset CSVs)
/// loads as it is: every platform resolves, the armour, ERA and APS fits and
/// row-level sources come across, and its munitions are kept apart.
#[test]
fn imported_csv_records_load() {
    let c = builtin();
    let imported: Vec<&Platform> = c.platforms().filter(|p| p.provenance == Provenance::Imported).collect();
    assert_eq!(imported.len(), 114);
    assert!(c.imported_munitions().count() >= 100);
    // Engine munitions with an importer id are built from that sourced row (weapon-fits.json).
    for m in c.munitions().filter(|m| m.id.starts_with("l7/")) {
        let row = c.imported_munition(&m.id).unwrap_or_else(|| panic!("{}: no sourced row", m.id));
        assert_eq!(m.penetration, row.penetration, "{}", m.id);
    }
    for p in &imported {
        // A reparented record also carries its parent's sources for what it inherits.
        assert!(p.sources.values().any(|s| s.kind == Provenance::Imported), "{}: imported with no row-level source", p.id);
    }
    // Turret and roof come across (the engine has no `turret` or `roof` aspect).
    let m1a1 = c.platform("l7/var_1_1").unwrap();
    assert_eq!(m1a1.body.armour.ke.front, 400.0);
    assert_eq!(m1a1.body.armour.ke.top, 25.0);
    assert_eq!(m1a1.body.armour.zones["turret"].ke.front, 450.0);
    // ERA kits: heavy ERA is ERA, a slat cage is not.
    let t90m = c.platforms().find(|p| p.provenance == Provenance::Imported && p.name == "T-90M Proryv").unwrap();
    assert!(t90m.body.era);
    assert_eq!(t90m.body.era_kind, EraKind::Heavy);
    let stryker = c.platforms().find(|p| p.provenance == Provenance::Imported && p.name == "General Dynamics Stryker").unwrap();
    assert!(!stryker.body.era);
    // Light ERA (Kontakt-1, ROMOR-A) works against shaped charges only.
    let cr2 = c.platform("l7/var_11_default").unwrap();
    assert!(cr2.body.era && cr2.body.era_kind == EraKind::Light);
    assert_eq!(c.platform("var_11_default").unwrap().body.era_kind, EraKind::Light);
    assert_eq!(c.platform("var_22_1").unwrap().body.era_kind, EraKind::Heavy);
    // A row for a vehicle the catalogue already has is parented on that record.
    let cr2 = c.platform("l7/var_11_default").unwrap();
    assert_eq!(cr2.parent.as_deref(), Some("var_11_default"));
    // Importer munitions keep their penetration and what it is measured against.
    let kornet_like = c.imported_munitions().filter(|m| m.pen_basis == Some(PenBasis::BehindEra)).count();
    assert!(kornet_like > 0);
}

/// The imported platforms carry their own weapons (weapon-fits.json, from the
/// capability CSV), not their generic parent's.
#[test]
fn imported_platforms_have_their_own_weapon_fits() {
    let c = builtin();
    let fires = |id: &str| -> Vec<String> { c.platform(id).unwrap().mounts.iter().flat_map(|m| m.munitions.clone()).collect() };
    // T-90M: a 125 mm gun with 125 mm rounds, no 120 mm, no IFV cannon.
    let t90m = fires("l7/var_161_1");
    assert!(t90m.iter().any(|m| m == "l7/m_3bm46"), "{t90m:?}");
    assert!(!t90m.iter().any(|m| m == "apfsds120" || m == "apfsds30"), "{t90m:?}");
    // Up-armoured HMMWV: a .50 cal only, not the generic IFV's cannon and ATGM.
    let hmmwv = c.platform("l7/var_115_default").unwrap();
    assert_eq!(hmmwv.mounts.len(), 1);
    assert_eq!(hmmwv.mounts[0].munitions, vec!["l7/m_m8_api".to_string()]);
    assert!(!hmmwv.stores.ammo.contains_key("apfsds30"), "the parent's rounds are dropped");
    // Uran-9: armed (its generic parent has no mounts).
    assert!(!c.platform("l7/var_1134_default").unwrap().mounts.is_empty());
    // Every fitted mount fires something the catalogue knows, and has rounds to fire.
    let fits: serde_json::Value = serde_json::from_str(include_str!("../../../data/catalogue/weapon-fits.json")).unwrap();
    let fitted = fits["fits"].as_object().unwrap();
    assert!(fitted.len() >= 100, "{}", fitted.len());
    for id in fitted.keys() {
        let p = c.platform(id).unwrap();
        for m in &p.mounts {
            assert!(!m.munitions.is_empty(), "{}: mount {} fires nothing", p.id, m.id);
            for mu in &m.munitions {
                assert!(c.munition(mu).is_some(), "{}: unknown munition {mu}", p.id);
                assert!(p.stores.ammo.get(mu).copied().unwrap_or(0.0) > 0.0, "{}: no {mu} carried", p.id);
            }
        }
    }
}

/// A fit is laid over a platform from another file; `null` removes what the parent has.
#[test]
fn a_fit_overlays_a_record_and_null_removes_from_the_parent() {
    let extra = r#"{
        "platforms": [ { "id": "t/child", "parent": "generic/mbt", "name": "Child" } ],
        "fits": { "t/child": { "mounts": [], "stores": { "ammo": { "apfsds120": null } } } }
    }"#;
    let c = Catalogue::from_sources(&sources_plus(extra)).unwrap();
    let p = c.platform("t/child").unwrap();
    assert!(p.mounts.is_empty());
    assert!(!p.stores.ammo.contains_key("apfsds120"));
    assert!(p.stores.ammo.contains_key("heat120"));
    let bad = r#"{ "fits": { "t/nobody": { "mounts": [] } } }"#;
    assert!(Catalogue::from_sources(&sources_plus(bad)).is_err());
}

/// Rounds say what their penetration figure allows for against ERA, so the
/// engine neither takes ERA off a behind-ERA figure again nor lets heavy ERA
/// degrade a rod made against it.
#[test]
fn rounds_carry_their_era_basis() {
    let c = builtin();
    let spike = c.munition("l7/m_spike_lr").unwrap();
    assert_eq!(spike.pen_basis, PenBasis::BehindEra);
    assert!(spike.tandem);
    let m829a3 = c.munition("l7/m_m829a3").unwrap();
    assert!(m829a3.defeats_era && m829a3.pen_basis == PenBasis::Rha);
    // Every behind-ERA engine round comes from a row that says so.
    for m in c.munitions().filter(|m| m.pen_basis == PenBasis::BehindEra) {
        assert_eq!(c.imported_munition(&m.id).and_then(|r| r.pen_basis), Some(PenBasis::BehindEra), "{}", m.id);
    }
    // Hand-built rounds are unchanged: read as RHA, not made against ERA.
    for id in ["kornet", "javelin", "apfsds125", "m829"] {
        let m = c.munition(id).unwrap();
        assert!(m.pen_basis == PenBasis::Rha && !m.defeats_era, "{id}");
    }
}
