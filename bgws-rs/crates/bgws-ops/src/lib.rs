//! Process and network models (RUST_ENGINE.md §3.1, §8.5, §8.7, §8.11–§8.13,
//! §6.4): the parts of the war that are decided by organisation, links and
//! depletion rather than by a single physical encounter.
//!
//! | Module | Plan | What it models |
//! |---|---|---|
//! | [`comms`] | §8.5 | bearers, nets, queues, channel access, mesh routing, PACE, crypto, RF emissions |
//! | [`drones`] | §8.7 | drone types, link dependence, link-loss autonomy, fibre limits, the FPV strike chain, stores, operators |
//! | [`counter_drone`] | §8.7 | detection by RF/radar/acoustic/EO, defeat by jammers, guns, interceptors, nets, DE, passive measures |
//! | [`ew`] | §8.7 | jammers as emitters, J/S at receivers, burn-through, radar noise and deception vs ECCM, signatures |
//! | [`pnt`] | §8.7 | GNSS C/N₀ under jamming, lock and reacquisition, spoofing, INS drift, munition CEP |
//! | [`logistics`] | §8.11 | supply network, min-cost flow, consumption, interdiction, last mile, maintenance |
//! | [`medical`] | §8.11 | the evacuation chain, triage queues, time-dependent mortality |
//! | [`c2`] | §8.12 | planning time, CCIR, battle rhythm, staff queues, HQ signatures and succession, structured orders |
//! | [`cyber`] | §8.12 | access campaigns and time-limited effects on named processes |
//! | [`aggregate`] | §8.13, §6.4 | stochastic Lanchester attrition, rate fitting, formations, (dis)aggregation |
//!
//! The crate depends only on `bgws-core`. Anything physical that another
//! crate computes (path loss, SINR at a receiver, J/S) comes in as an input
//! or through a small trait defined here, so the models stay testable alone.
//!
//! Every number a model uses is a declared figure (rule 3): [`figures`] is
//! the union of every module's declarations.

pub mod aggregate;
pub mod c2;
pub mod comms;
pub mod counter_drone;
pub mod cyber;
pub mod drones;
pub mod ew;
pub mod logistics;
pub mod math;
pub mod medical;
pub mod pnt;

use bgws_core::Figure;

/// Every figure this crate declares (rule 3), from all modules.
pub fn figures() -> Vec<Figure> {
    let mut v = Vec::new();
    v.extend(comms::figures());
    v.extend(drones::figures());
    v.extend(counter_drone::figures());
    v.extend(ew::figures());
    v.extend(pnt::figures());
    v.extend(logistics::figures());
    v.extend(medical::figures());
    v.extend(c2::figures());
    v.extend(cyber::figures());
    v.extend(aggregate::figures());
    v
}

/// A `Figures` registry holding exactly this crate's figures. Convenient for
/// tests and for hosts that run an ops model on its own.
pub fn default_figures() -> bgws_core::Figures {
    bgws_core::Figures::from_list(figures()).expect("bgws-ops figures are consistent")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn figures_build_a_registry() {
        let f = bgws_core::Figures::from_list(figures()).expect("no duplicates, all in range");
        assert!(f.len() > 100);
        for fig in f.iter() {
            assert!(!fig.unit.is_empty(), "{} has no unit", fig.key);
            assert!(!fig.note.is_empty(), "{} has no note", fig.key);
            assert!(fig.min <= fig.max, "{}", fig.key);
        }
    }
}
