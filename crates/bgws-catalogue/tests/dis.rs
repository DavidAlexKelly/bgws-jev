//! Every platform and munition carries a DIS type (SISO-REF-010) that fits
//! what it is: the kind and domain agree with the catalogue's domain, the
//! category is one the SISO tables define for that kind and domain, and a
//! guided munition is in the Guided category.

use bgws_catalogue::{Catalogue, Domain, Guidance};

/// Categories SISO-REF-010 v35 defines, by (kind, domain): only the values
/// the catalogue uses are listed, so a typo or an unchecked new value fails.
fn known(kind: u8, domain: u8, category: u8) -> bool {
    let ok: &[u8] = match (kind, domain) {
        // Platform-Land Category (uid 9)
        (1, 1) => &[1, 2, 3, 4, 7, 10, 28, 29, 39, 50],
        // Platform-Air Category (uid 10)
        (1, 2) => &[1, 2, 3, 4, 5, 8, 20, 21, 50],
        // Platform-Surface Category (uid 11)
        (1, 3) => &[4, 6, 19, 50, 81],
        // Platform-Subsurface Category (uid 12)
        (1, 4) => &[3, 5],
        // Platform-Space Category (uid 13)
        (1, 5) => &[12, 13, 16],
        // Life Form Land Categories (uid 472): Conventional Armed Forces
        (3, 1) => &[10],
        // Cultural feature, land (entity tree 5.1.0)
        (5, 1) => &[2, 10, 13],
        // Sensor/Emitter Category (uid 28)
        (9, 1) => &[6],
        // Munition Category (uid 15), any munition domain 0-12 (uid 14)
        (2, 0..=12) => &[1, 2, 3],
        _ => &[],
    };
    ok.contains(&category)
}

#[test]
fn every_platform_has_a_fitting_dis_type() {
    let c = Catalogue::builtin().expect("the catalogue loads");
    let mut missing = Vec::new();
    for p in c.platforms() {
        let Some(d) = p.dis else {
            // Only what has no body to show goes unpublished.
            if p.domain != Domain::Cyber && p.class != "cyberUnit" {
                missing.push(p.id.clone());
            }
            continue;
        };
        let fits = match p.domain {
            // A platform, a life form, a sensor or (for structures) a cultural feature, on land.
            Domain::Land => matches!((d.kind, d.domain), (1, 1) | (3, 1) | (9, 1)),
            Domain::Air => (d.kind, d.domain) == (1, 2),
            Domain::Surface => (d.kind, d.domain) == (1, 3),
            Domain::Subsurface => (d.kind, d.domain) == (1, 4),
            Domain::Space => (d.kind, d.domain) == (1, 5),
            Domain::Node => (d.kind, d.domain) == (5, 1),
            Domain::Munition | Domain::Cyber => false,
        };
        assert!(fits, "{}: DIS kind {} domain {} does not fit catalogue domain {:?}", p.id, d.kind, d.domain, p.domain);
        assert!(known(d.kind, d.domain, d.category), "{}: category {} is not a known SISO-REF-010 value for kind {} domain {}", p.id, d.category, d.kind, d.domain);
        // People on foot are life forms; vehicles are not.
        let foot = matches!(p.body.vulnerability, bgws_catalogue::Vulnerability::Personnel);
        assert_eq!(foot, d.kind == 3, "{}: on foot {} but DIS kind {}", p.id, foot, d.kind);
    }
    assert!(missing.is_empty(), "platforms without a DIS type: {missing:?}");
}

#[test]
fn every_munition_has_a_fitting_dis_type() {
    let c = Catalogue::builtin().expect("the catalogue loads");
    for m in c.munitions() {
        let d = m.dis.unwrap_or_else(|| panic!("{}: no DIS type", m.id));
        assert_eq!(d.kind, 2, "{}: a munition is DIS kind 2", m.id);
        assert!(known(d.kind, d.domain, d.category), "{}: domain {} category {} not known", m.id, d.domain, d.category);
        // A radar-directed shell (proximity or AHEAD fuze) is aimed by fire
        // control, not guided in flight: Ballistic.
        let fire_control = m.kind == bgws_catalogue::MunitionKind::Shell && m.guidance == Guidance::Radar;
        let guided = m.guidance != Guidance::None && !fire_control;
        assert_eq!(guided, d.category == 1, "{}: guidance {:?} but DIS category {}", m.id, m.guidance, d.category);
    }
}
