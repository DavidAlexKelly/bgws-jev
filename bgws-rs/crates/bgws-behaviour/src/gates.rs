//! Hard gates (RUST_ENGINE.md §10.1, design rule 5; DECISION_FLOW_GRAPHS.md §2
//! "action masking"): rules of engagement, weapon release authority, EMCON,
//! capability and cohesion are never left to a model's judgement **[ref R8]**.
//! They are applied to every option before it is offered, so Jev never sees a
//! removed option; each drop records why as `gate:<name>`.
//!
//! The gates read only an action's traits (`library::ActionTraits`), never the
//! world, so the caller fills a [`Gates`] from the agent's orders and its own
//! state and the walk stays pure.

use crate::library::{ActionTraits, Capability};
use crate::types::GateName;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Rules of engagement in the agent's orders (as the TypeScript `UnitOrders.roe`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Roe {
    /// Weapons hold: do not fire at all ("never").
    Never,
    /// Fire only once the enemy has fired on your side.
    IfFiredUpon,
    /// Fire at anything that closes to short range (range is the sim's check).
    WithinShortRange,
    /// Fire at anything in reach.
    #[default]
    Always,
}

/// Weapon release authority (Command: Modern Operations' doctrine setting).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WeaponRelease {
    /// No weapon is released.
    Hold,
    /// Only at targets positively identified as hostile.
    Tight,
    /// At any target not identified as friendly.
    #[default]
    Free,
}

/// Emission control.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Emcon {
    #[default]
    Unrestricted,
    /// No active emitters (radar, jamming); radio allowed.
    Restricted,
    /// Nothing radiates: no emitters and no transmissions.
    Silent,
}

/// The agent's cohesion state (§8.10).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Cohesion {
    #[default]
    Steady,
    /// Will not close with the enemy.
    Shaken,
    /// Only steps back: withdraws, hides, holds.
    Broken,
}

/// Everything the hard gates read, for one agent at one decision.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Gates {
    pub roe: Roe,
    /// The agent's side has been fired upon (for `Roe::IfFiredUpon`).
    #[serde(default)]
    pub fired_upon: bool,
    pub weapon_release: WeaponRelease,
    /// The target of this decision is positively identified (for `WeaponRelease::Tight`).
    #[serde(default)]
    pub target_identified: bool,
    pub emcon: Emcon,
    /// What the agent can do now; `None` = not known, nothing masked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<BTreeSet<Capability>>,
    pub cohesion: Cohesion,
}

impl Gates {
    /// No gate closed: every option passes (what the TypeScript walker does).
    pub fn open() -> Gates {
        Gates::default()
    }

    /// Check an action against every gate, in a fixed order (ROE, weapon
    /// release, EMCON, capability, cohesion); the first that closes names the drop.
    pub fn check(&self, t: &ActionTraits) -> Result<(), GateName> {
        if t.fires {
            let roe_closed = match self.roe {
                Roe::Never => true,
                Roe::IfFiredUpon => !self.fired_upon,
                Roe::WithinShortRange | Roe::Always => false,
            };
            if roe_closed {
                return Err(GateName::Roe);
            }
            let wra_closed = match self.weapon_release {
                WeaponRelease::Hold => true,
                WeaponRelease::Tight => !self.target_identified,
                WeaponRelease::Free => false,
            };
            if wra_closed {
                return Err(GateName::WeaponRelease);
            }
        }
        let emcon_closed = match self.emcon {
            Emcon::Unrestricted => false,
            Emcon::Restricted => t.emits,
            Emcon::Silent => t.emits || t.transmits,
        };
        if emcon_closed {
            return Err(GateName::Emcon);
        }
        if let Some(have) = &self.capabilities {
            if t.required().iter().any(|c| !have.contains(c)) {
                return Err(GateName::Capability);
            }
        }
        let cohesion_closed = match self.cohesion {
            Cohesion::Steady => false,
            Cohesion::Shaken => t.assault,
            Cohesion::Broken => !t.deescalates,
        };
        if cohesion_closed {
            return Err(GateName::Cohesion);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(fires: bool, emits: bool, transmits: bool, assault: bool, deescalates: bool) -> ActionTraits {
        ActionTraits { fires, emits, transmits, moves: false, assault, deescalates, requires: vec![] }
    }

    #[test]
    fn each_gate_closes_on_its_own_trait() {
        let fire = t(true, false, false, false, false);
        assert_eq!(Gates::open().check(&fire), Ok(()));
        assert_eq!(Gates { roe: Roe::Never, ..Gates::open() }.check(&fire), Err(GateName::Roe));
        assert_eq!(Gates { roe: Roe::IfFiredUpon, ..Gates::open() }.check(&fire), Err(GateName::Roe));
        assert_eq!(Gates { roe: Roe::IfFiredUpon, fired_upon: true, ..Gates::open() }.check(&fire), Ok(()));
        assert_eq!(Gates { weapon_release: WeaponRelease::Tight, ..Gates::open() }.check(&fire), Err(GateName::WeaponRelease));
        assert_eq!(Gates { weapon_release: WeaponRelease::Tight, target_identified: true, ..Gates::open() }.check(&fire), Ok(()));
        assert_eq!(Gates { emcon: Emcon::Restricted, ..Gates::open() }.check(&t(false, false, true, false, false)), Ok(()));
        assert_eq!(Gates { emcon: Emcon::Silent, ..Gates::open() }.check(&t(false, false, true, false, false)), Err(GateName::Emcon));
        assert_eq!(Gates { emcon: Emcon::Restricted, ..Gates::open() }.check(&t(false, true, false, false, false)), Err(GateName::Emcon));
        let caps = Gates { capabilities: Some(BTreeSet::new()), ..Gates::open() };
        assert_eq!(caps.check(&fire), Err(GateName::Capability));
        assert_eq!(caps.check(&t(false, false, false, false, true)), Ok(()));
        assert_eq!(Gates { cohesion: Cohesion::Shaken, ..Gates::open() }.check(&t(true, false, false, true, false)), Err(GateName::Cohesion));
        assert_eq!(Gates { cohesion: Cohesion::Broken, ..Gates::open() }.check(&fire), Err(GateName::Cohesion));
        assert_eq!(Gates { cohesion: Cohesion::Broken, ..Gates::open() }.check(&t(false, false, false, false, true)), Ok(()));
    }
}
