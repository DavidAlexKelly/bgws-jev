//! Walk invariants under any gates and any removals: every option is either
//! offered or dropped (once, in order); nothing offered breaks a gate; the
//! fallback is offered or absent; the same input walks the same way.

use bgws_behaviour::facts::NoFacts;
use bgws_behaviour::library::Capability;
use bgws_behaviour::types::{GraphNode, Template, WalkResult};
use bgws_behaviour::{Cohesion, Emcon, Gates, LoadMode, Registry, Roe, Vocabulary, WeaponRelease};
use proptest::prelude::*;

fn gates() -> impl Strategy<Value = Gates> {
    (0..4u8, any::<bool>(), 0..3u8, any::<bool>(), 0..3u8, proptest::option::of(proptest::collection::btree_set(0..10usize, 0..10)), 0..3u8).prop_map(|(roe, fired, wra, ident, emcon, caps, coh)| Gates {
        roe: [Roe::Never, Roe::IfFiredUpon, Roe::WithinShortRange, Roe::Always][roe as usize],
        fired_upon: fired,
        weapon_release: [WeaponRelease::Hold, WeaponRelease::Tight, WeaponRelease::Free][wra as usize],
        target_identified: ident,
        emcon: [Emcon::Unrestricted, Emcon::Restricted, Emcon::Silent][emcon as usize],
        capabilities: caps.map(|s| s.into_iter().map(|i| Capability::ALL[i]).collect()),
        cohesion: [Cohesion::Steady, Cohesion::Shaken, Cohesion::Broken][coh as usize],
    })
}

const TRIGGERS: [&str; 8] = ["D1", "D2", "D6", "D10", "vampireInbound", "linkDegraded", "jammingOrder", "trackAppears"];

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    #[test]
    fn walks_partition_and_respect_gates(g in gates(), trig in 0..TRIGGERS.len(), remove in proptest::collection::vec(prop::sample::select(vec!["engage", "observe", "returnFire", "assault", "climb", "silent", "hold"]), 0..3)) {
        let reg = Registry::builtin().unwrap();
        let variant = Template { remove: Some(remove.iter().map(|s| s.to_string()).collect()), ..Template::variant("custom/p", "core/default") };
        let reg = Registry::with_custom(&[variant], &Vocabulary::builtin(), LoadMode::Strict).unwrap_or(reg);
        let trigger = TRIGGERS[trig];
        let t = reg.template_for_trigger(&["custom/p", "core/default"], trigger).unwrap();
        let w = reg.walk(&t, trigger, &NoFacts, &g).unwrap();
        prop_assert_eq!(&w, &reg.walk(&t, trigger, &NoFacts, &g).unwrap());
        let WalkResult::Choose { node_id, options, dropped, fallback, .. } = w else { return Ok(()) };
        let Some(GraphNode::Choose { options: all, .. }) = t.nodes.get(&node_id) else { panic!() };
        prop_assert_eq!(options.len() + dropped.len(), all.len());
        let offered: Vec<_> = options.iter().map(|o| o.id.clone()).collect();
        let expected: Vec<_> = all.iter().map(|o| o.id.clone()).filter(|id| !dropped.iter().any(|d| &d.id == id)).collect();
        prop_assert_eq!(&offered, &expected);
        for o in &options {
            prop_assert_eq!(g.check(&reg.vocabulary().action(&o.action).unwrap().traits), Ok(()));
            prop_assert!(!t.remove.contains(&o.id));
        }
        match fallback {
            Some(f) => prop_assert!(offered.contains(&f)),
            None => prop_assert!(offered.is_empty()),
        }
    }
}
