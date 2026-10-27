//! The action library (RUST_ENGINE.md §10.1; DECISION_FLOW_GRAPHS.md §3.2):
//! every action a template may name, with its label and description, and the
//! traits the hard gates read (does it fire, radiate, transmit, move, close
//! with the enemy, or step back from a fight?).
//!
//! A port of the vocabulary halves of `behaviour/actions/land.ts` (the 53
//! land verbs, labelled as the Behaviour Manager labels them) and
//! `behaviour/actions/world.ts` (the world verbs and their labels). The
//! engine-side handlers (what each verb does to the world) belong to the sim;
//! this is only what a template, a gate and a decider need to know.

use serde::{Deserialize, Serialize};

/// Something an agent must have for an action to be offered (§10.1 capability gate).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Capability {
    /// A weapon that can fire (ammunition left, not destroyed).
    Weapon,
    /// It can move.
    Mobility,
    /// A working radio on a net.
    Radio,
    /// An active emitter (radar, jammer) that EMCON governs.
    Emitter,
    Jammer,
    Radar,
    /// Drones in stores or under control.
    Drones,
    FibreDrones,
    /// Infantry carried, able to dismount or remount.
    Dismounts,
    /// Autonomous terminal guidance.
    Autonomy,
}

impl Capability {
    pub const ALL: [Capability; 10] = [
        Capability::Weapon,
        Capability::Mobility,
        Capability::Radio,
        Capability::Emitter,
        Capability::Jammer,
        Capability::Radar,
        Capability::Drones,
        Capability::FibreDrones,
        Capability::Dismounts,
        Capability::Autonomy,
    ];
}

/// The domain whose decision point offers an action.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Domain {
    Land,
    Fires,
    C2,
    Drones,
    Ew,
    Air,
    Sea,
    Space,
    Cyber,
    Logistics,
    Medical,
    Killweb,
}

/// What an action does, as far as the hard gates and the safeguards care.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ActionTraits {
    /// Releases a weapon (ROE and weapon release authority govern it).
    pub fires: bool,
    /// Radiates actively: radar, jamming (EMCON governs it).
    pub emits: bool,
    /// Transmits on a radio net (EMCON silent forbids it).
    pub transmits: bool,
    pub moves: bool,
    /// Closes with the enemy or presses an attack (a shaken unit will not).
    pub assault: bool,
    /// Steps back from escalation: withdraws, hides, holds fire, evades (§10.6).
    pub deescalates: bool,
    /// Capabilities beyond those implied above.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<Capability>,
}

impl ActionTraits {
    /// Everything the action needs: the implied capabilities, then the extras.
    pub fn required(&self) -> Vec<Capability> {
        let mut out = Vec::new();
        if self.fires {
            out.push(Capability::Weapon);
        }
        if self.moves {
            out.push(Capability::Mobility);
        }
        if self.transmits {
            out.push(Capability::Radio);
        }
        if self.emits {
            out.push(Capability::Emitter);
        }
        for c in &self.requires {
            if !out.contains(c) {
                out.push(*c);
            }
        }
        out
    }
}

/// One action in the library.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionInfo {
    pub name: String,
    pub domain: Domain,
    /// A few words, as the Behaviour Manager and a decider read it.
    pub label: String,
    /// One sentence on what it offers.
    pub description: String,
    pub traits: ActionTraits,
}

// Trait flags for the table below.
const F: u8 = 1; // fires
const E: u8 = 2; // emits
const T: u8 = 4; // transmits
const M: u8 = 8; // moves
const A: u8 = 16; // assault
const D: u8 = 32; // de-escalates

fn traits(flags: u8, requires: &[Capability]) -> ActionTraits {
    ActionTraits {
        fires: flags & F != 0,
        emits: flags & E != 0,
        transmits: flags & T != 0,
        moves: flags & M != 0,
        assault: flags & A != 0,
        deescalates: flags & D != 0,
        requires: requires.to_vec(),
    }
}

/// "land.pullBackToFriends" → "Pull back to friends" (the Behaviour Manager's `actionLabel`).
pub fn label_from_name(action: &str) -> String {
    let base = action.strip_prefix("land.").unwrap_or(action);
    let mut words = String::new();
    let mut prev_lower = false;
    for ch in base.chars() {
        if ch.is_ascii_uppercase() && prev_lower {
            words.push(' ');
        }
        prev_lower = ch.is_ascii_lowercase();
        words.push(ch.to_ascii_lowercase());
    }
    let mut out = String::new();
    let mut chars = words.chars();
    if let Some(first) = chars.next() {
        out.extend(first.to_uppercase());
        out.push_str(chars.as_str());
    }
    out
}

/// The 53 land actions, in `LAND_ACTIONS` order: name, flags, extra requirements, description.
const LAND: &[(&str, u8, &[Capability], &str)] = &[
    // D0: new orders arrive mid-fight.
    ("land.complyWithOrders", M, &[], "Start the new orders now (breaking off at once if they are urgent)."),
    ("land.breakOffCovered", M | D, &[], "Break off under cover, then start the new orders."),
    ("land.fireAndBack", F | M | D, &[], "Fire one more time, then pull back and start the new orders."),
    ("land.finishFightFirst", F, &[], "Finish the fight in hand first, then start the new orders."),
    ("land.breakContactFirst", M | D, &[], "Break contact first, then start the new orders."),
    // D1: new contact.
    ("land.carryOn", 0, &[], "Carry on with what it was doing."),
    ("land.engageNow", F, &[], "Engage the enemy now, from here."),
    ("land.waitFor", F, &[], "Hold fire and wait for a better moment (closer, or a flank), then fire."),
    ("land.betterPosition", F | M, &[], "Move to a better firing position (hull-down, closer, a flank), then engage."),
    ("land.observe", D, &[], "Stay still, hold fire and watch."),
    ("land.pullBackToFriends", M | D, &[], "Pull back toward friends, out of the enemy's reach."),
    // D2: fired on, shooter located.
    ("land.returnFire", F, &[], "Return fire on the shooter."),
    ("land.hullDownThenFire", F | M, &[], "Move hull-down, then return fire."),
    ("land.outOfSight", M | D, &[], "Pull back out of the enemy's sight."),
    ("land.askFriendForHelp", T, &[], "Ask a friend in reach to fire on the shooter."),
    ("land.assault", F | M | A, &[], "Assault the shooter: close and fight through."),
    ("land.goQuietInCover", D, &[], "Stop firing and go quiet in cover."),
    // D3: fired on, shooter not located.
    ("land.searchBearing", 0, &[], "Search the bearing the fire came from."),
    ("land.coverFromBearing", M | D, &[], "Move into cover from that bearing."),
    ("land.pullBackFromBearing", M | D, &[], "Pull back away from that bearing."),
    // D4: trigger met.
    ("land.fireNow", F, &[], "Fire now: the moment it waited for has come."),
    ("land.waitCloser", F, &[], "Wait until the enemy is closer, then fire."),
    ("land.letPass", D, &[], "Let it pass unengaged and stay hidden."),
    // D5: enemy behaviour changed.
    ("land.fireFirst", F, &[], "Fire first, before the enemy does."),
    ("land.keepHolding", D, &[], "Keep holding still in hiding."),
    ("land.relocateQuietly", M | D, &[], "Relocate quietly to another hidden spot."),
    ("land.slipAway", M | D, &[], "Slip away out of sight."),
    // D6: after a volley.
    ("land.keepFiring", F, &[], "Keep firing from here."),
    ("land.fireAndMove", F | M, &[], "Move to a new firing position before the enemy finds this one, then fire again."),
    ("land.shiftFire", F, &[], "Shift fire to another target."),
    ("land.goQuiet", D, &[], "Stop firing and go quiet."),
    // D7: target lost.
    ("land.resumeOrCarryOn", 0, &[], "Resume its orders, or carry on."),
    ("land.watchLastSeen", D, &[], "Watch where the target was last seen."),
    ("land.regainContact", M, &[], "Move to regain contact."),
    ("land.shiftFireFromLost", F, &[], "Shift fire to another target it can see."),
    // D8: fire not working.
    ("land.betterShotOnTarget", F | M, &[], "Move to a better shot on its target."),
    ("land.shiftFireFromTarget", F, &[], "Shift fire to a target it can hurt."),
    ("land.stopFiring", D, &[], "Stop firing and go quiet."),
    ("land.outOfTargetSight", M | D, &[], "Pull back out of the target's sight."),
    ("land.keepAtIt", F, &[], "Keep firing on its target."),
    // D9: friend needs help.
    ("land.helpFriend", F, &[], "Fire on the friend's attacker."),
    ("land.moveToSupport", M, &[], "Move to support the friend."),
    // D12: request received.
    ("land.complyWithRequest", F, &[], "Do as the friend asks: cover it or fire on its enemy."),
    ("land.partlyOverwatch", F, &[], "Partly comply: overwatch from where it is."),
    ("land.cannotComply", D, &[], "Reply that it cannot comply."),
    // D10: order step done or blocked.
    ("land.resumeOrders", M, &[], "Resume its orders."),
    ("land.skipToNextPhase", M, &[], "Skip to the next step of its orders."),
    ("land.holdHere", D, &[], "Hold here."),
    ("land.overwatchHere", F, &[], "Overwatch from here: fire at anything in reach."),
    ("land.keepHoldingHere", D, &[], "Already holding: keep holding here."),
    // D11: rallied.
    ("land.rejoin", M, &[], "Rejoin: resume its orders."),
    ("land.holdAndRecover", D, &[], "Hold and recover."),
    ("land.joinNearestFriends", M | D, &[], "Join the nearest steady friends."),
];

use Capability as C;
use Domain::*;

/// The world actions, in `WORLD_ACTION_INFO` order: name, domain, label, flags, extra requirements.
const WORLD: &[(&str, Domain, &str, u8, &[Capability])] = &[
    // W3 land: fire requests, counter-battery, obstacles, dismounts, stocks.
    ("fires.fireNow", Fires, "Fire the mission now", F, &[]),
    ("fires.fireWhenLaid", Fires, "Lay on the target carefully, then fire", F, &[]),
    ("fires.refuse", Fires, "Refuse the mission (out of range, out of ammunition)", D, &[]),
    ("fires.fireThenScoot", Fires, "Fire, then move at once (shoot and scoot)", F | M, &[]),
    ("fires.displaceNow", Fires, "Displace now: the position may be located", M | D, &[]),
    ("fires.finishThenDisplace", Fires, "Finish the mission, then displace", F | M, &[]),
    ("fires.stay", Fires, "Stay in position", 0, &[]),
    ("land.breach", Land, "Breach the obstacle", M, &[]),
    ("land.bypass", Land, "Find a way round the obstacle", M, &[]),
    ("land.callEngineers", Land, "Halt and call the engineers", T | D, &[]),
    ("land.waitAtObstacle", Land, "Wait at the obstacle", D, &[]),
    ("land.pushThroughLane", Land, "Push through on a lane", M, &[]),
    ("land.reverseOut", Land, "Reverse out of the minefield", M | D, &[]),
    ("land.dismountHere", Land, "Dismount the infantry here", 0, &[C::Dismounts]),
    ("land.dismountInCover", Land, "Dismount in cover close by", M, &[C::Dismounts]),
    ("land.stayMounted", Land, "Stay mounted", 0, &[]),
    ("land.remount", Land, "Remount the infantry", 0, &[C::Dismounts]),
    ("land.requestResupply", Land, "Ask for resupply", T, &[]),
    ("land.conserve", Land, "Conserve: fire only in self-defence", D, &[]),
    ("land.withdrawToResupply", Land, "Withdraw to resupply", M | D, &[]),
    ("land.digIn", Land, "Dig in here", D, &[]),
    // W4 C2.
    ("c2.carryOnPhase", C2, "Carry on with the current phase", 0, &[]),
    ("c2.actOnIntent", C2, "Act on the commander's intent: go on to the next phase", M, &[]),
    ("c2.holdAndReestablish", C2, "Hold and try to re-establish comms", T | D, &[]),
    ("c2.moveToRegainComms", C2, "Move to regain comms with HQ", M, &[]),
    ("c2.fireFirstRequest", C2, "Fire the most urgent request first", F, &[]),
    ("c2.commitReserve", C2, "Commit the reserve", M | A, &[]),
    ("c2.holdReserve", C2, "Hold the reserve", D, &[]),
    ("c2.reinforce", C2, "Reinforce the unit in trouble", M, &[]),
    ("c2.swapWithReserve", C2, "Swap it with the reserve", M, &[]),
    ("c2.withdrawIt", C2, "Withdraw it", M | D, &[]),
    ("c2.carryOn", C2, "Carry on as planned", 0, &[]),
    ("c2.displacePost", C2, "Displace the command post", M | D, &[]),
    ("c2.alternateBearer", C2, "Switch to the alternate bearer", T, &[]),
    ("c2.acceptRisk", C2, "Accept the risk and stay on the air", T, &[]),
    // W5 drones.
    ("drones.reportAndWatch", Drones, "Report it and keep watching", T | D, &[]),
    ("drones.callFire", Drones, "Call fire on it", T, &[]),
    ("drones.handOff", Drones, "Hand it off to a strike team", T, &[]),
    ("drones.strike", Drones, "Strike it", F, &[C::Drones]),
    ("drones.pushOn", Drones, "Push on (risk losing the drone)", M, &[C::Drones]),
    ("drones.climbForLink", Drones, "Climb to regain the link", M, &[C::Drones]),
    ("drones.returnHome", Drones, "Bring it home", M | D, &[C::Drones]),
    ("drones.goAutonomous", Drones, "Switch to autonomous terminal guidance", 0, &[C::Drones, C::Autonomy]),
    ("drones.onePass", Drones, "One more pass", M, &[C::Drones]),
    ("drones.ditchOnTarget", Drones, "Ditch it on the target", F, &[C::Drones]),
    ("drones.switchToFibre", Drones, "Switch to fibre drones", 0, &[C::FibreDrones]),
    ("drones.relocate", Drones, "Relocate the team", M | D, &[]),
    ("drones.callFireOnJammer", Drones, "Call fire on the jammer", T, &[]),
    ("drones.wait", Drones, "Wait it out", D, &[]),
    ("drones.finishThenRelocate", Drones, "Finish the flight, then relocate", M, &[]),
    ("drones.stay", Drones, "Stay put", 0, &[]),
    ("drones.freezeAndHide", Drones, "Freeze and hide from the drone", D, &[]),
    ("drones.shootItDown", Drones, "Shoot it down", F, &[]),
    ("drones.scatter", Drones, "Scatter", M | D, &[]),
    ("drones.moveToCover", Drones, "Move to cover", M | D, &[]),
    ("drones.callCounterDrone", Drones, "Call counter-drone", T, &[]),
    ("drones.engageTarget", Drones, "Engage this target", F, &[]),
    ("drones.holdForBetter", Drones, "Hold for a better target", D, &[]),
    // W6 EW.
    ("ew.locate", Ew, "Locate the emitter (takes time)", 0, &[]),
    ("ew.jam", Ew, "Jam it", E, &[C::Jammer]),
    ("ew.reportForStrike", Ew, "Report it for a strike", T, &[]),
    ("ew.ignore", Ew, "Ignore it", D, &[]),
    ("ew.goSilent", Ew, "Go silent", D, &[]),
    ("ew.relocateAfter", Ew, "Relocate after this transmission", M | D, &[]),
    ("ew.carryOn", Ew, "Carry on transmitting", T, &[]),
    ("ew.switchBearer", Ew, "Switch bearer", T, &[]),
    ("ew.jamSector", Ew, "Jam the sector", E, &[C::Jammer]),
    ("ew.protectAdvance", Ew, "Move up behind the advance and protect it", E | M, &[C::Jammer]),
    ("ew.fireAnyway", Ew, "Fire anyway", F, &[]),
    ("ew.switchMunition", Ew, "Switch to non-GNSS munitions", F, &[]),
    ("ew.waitForPnt", Ew, "Wait for PNT to recover", D, &[]),
    ("ew.requestDroneGuidance", Ew, "Ask for drone guidance onto the target", T, &[]),
    // W7 air.
    ("air.defend", Air, "Defend: notch, chaff and flares", M | D, &[]),
    ("air.pressAttack", Air, "Press the attack", F | A, &[]),
    ("air.abort", Air, "Abort the mission", M | D, &[]),
    ("air.shootNow", Air, "Shoot now", F, &[]),
    ("air.closeForPk", Air, "Close for a surer shot", F | M | A, &[]),
    ("air.holdFire", Air, "Hold fire (not identified, or the rules forbid it)", D, &[]),
    ("air.rtb", Air, "Return to base", M | D, &[]),
    ("air.tanker", Air, "Go to the tanker", M, &[]),
    ("air.onePass", Air, "Stay for one more pass", F, &[]),
    ("air.goAround", Air, "Route around the threat", M | D, &[]),
    ("air.goLow", Air, "Go low under the radar", M, &[]),
    ("air.requestSead", Air, "Request SEAD", T, &[]),
    ("air.engageCheapest", Air, "Engage with the cheapest interceptor that will do", F, &[]),
    ("air.engageSurest", Air, "Engage with the surest interceptor", F, &[]),
    ("air.holdTight", Air, "Hold: weapons tight, not identified", D, &[]),
    ("air.goSilentLetPass", Air, "Go silent and let it pass", D, &[]),
    ("air.shutDown", Air, "Shut the radar down", D, &[]),
    ("air.stayUp", Air, "Stay up and engage", F | E, &[C::Radar]),
    ("air.relocateRadar", Air, "Relocate the radar", M | D, &[]),
    ("air.acceptNineLine", Air, "Accept the 9-line", F, &[]),
    ("air.acceptWithDelay", Air, "Accept, with delay", F, &[]),
    ("air.declineNineLine", Air, "Decline (threat or fuel)", D, &[]),
    // W8 sea.
    ("sea.engageLong", Sea, "Engage with the long-range layer", F, &[]),
    ("sea.engageMedium", Sea, "Engage with the medium layer", F, &[]),
    ("sea.engageShort", Sea, "Leave it to point defence", F, &[]),
    ("sea.decoyManoeuvre", Sea, "Decoys and manoeuvre", M | D, &[]),
    ("sea.unmask", Sea, "Turn to unmask more launchers", F | M, &[]),
    ("sea.leaveToReload", Sea, "Leave station to reload", M | D, &[]),
    ("sea.conserve", Sea, "Stay and conserve: point defence only", D, &[]),
    ("sea.requestEscort", Sea, "Request an escort", T, &[]),
    ("sea.engageBeyondHorizon", Sea, "Engage beyond the horizon", F, &[]),
    ("sea.closeToIdentify", Sea, "Close to identify", M, &[]),
    ("sea.shadow", Sea, "Shadow it", M, &[]),
    ("sea.avoid", Sea, "Avoid it", M | D, &[]),
    ("sea.attack", Sea, "Attack", F | A, &[]),
    ("sea.track", Sea, "Track it", 0, &[]),
    ("sea.evade", Sea, "Evade: go deep, sprint and drift", M | D, &[]),
    ("sea.goSilent", Sea, "Go silent", D, &[]),
    ("sea.gunsAndHelo", Sea, "Engage with guns and helicopter", F, &[]),
    ("sea.withdraw", Sea, "Withdraw", M | D, &[]),
    // W9 space and cyber.
    ("space.imageArea", Space, "Image this area on the pass", 0, &[]),
    ("space.skipPass", Space, "Hold the pass for a better area", 0, &[]),
    ("cyber.executeNow", Cyber, "Execute the access now", A, &[]),
    ("cyber.holdForMoment", Cyber, "Hold for a better moment", D, &[]),
    ("cyber.abandon", Cyber, "Abandon it: it may be detected", D, &[]),
    // W10 logistics and medical.
    ("logistics.convoy", Logistics, "Send a convoy", M, &[]),
    ("logistics.ugv", Logistics, "Send a ground robot", M, &[]),
    ("logistics.drone", Logistics, "Send it by drone", 0, &[C::Drones]),
    ("logistics.waitForDark", Logistics, "Wait for dark", D, &[]),
    ("logistics.pushOn", Logistics, "Push on", M, &[]),
    ("logistics.alternateRoute", Logistics, "Take the alternate route", M, &[]),
    ("logistics.scatter", Logistics, "Scatter", M | D, &[]),
    ("logistics.takeCover", Logistics, "Take cover", D, &[]),
    ("logistics.relocateNode", Logistics, "Relocate the node back", M | D, &[]),
    ("logistics.disperseStocks", Logistics, "Disperse the stocks", D, &[]),
    ("logistics.repairAndStay", Logistics, "Repair and stay", 0, &[]),
    ("medical.evacuateNow", Medical, "Evacuate now under threat", M, &[]),
    ("medical.waitForDark", Medical, "Wait for dark", D, &[]),
    ("medical.requestUgv", Medical, "Ask for a robot or drone to evacuate", T, &[]),
    // W11 kill web.
    ("killweb.useShooter", Killweb, "Task this shooter", F, &[]),
    ("killweb.defer", Killweb, "Defer the target", D, &[]),
    ("killweb.restrike", Killweb, "Strike again", F, &[]),
    ("killweb.lookAgain", Killweb, "Look again before deciding", D, &[]),
    ("killweb.moveOn", Killweb, "Move on to the next target", 0, &[]),
    ("killweb.holdGuns", Killweb, "Hold the guns", D, &[]),
    ("killweb.shiftAircraft", Killweb, "Shift the aircraft", 0, &[]),
    ("killweb.acceptRisk", Killweb, "Accept the risk", F, &[]),
];

/// The land action names, in `LAND_ACTIONS` order.
pub fn land_action_names() -> Vec<&'static str> {
    LAND.iter().map(|(n, ..)| *n).collect()
}

/// The world action names, in `WORLD_ACTIONS` order.
pub fn world_action_names() -> Vec<&'static str> {
    WORLD.iter().map(|(n, ..)| *n).collect()
}

/// The land actions with their labels, descriptions and traits.
pub fn land_actions() -> Vec<ActionInfo> {
    LAND.iter()
        .map(|(name, flags, req, description)| ActionInfo {
            name: (*name).into(),
            domain: Domain::Land,
            label: label_from_name(name),
            description: (*description).into(),
            traits: traits(*flags, req),
        })
        .collect()
}

/// The world actions with their labels and traits. The label is also the
/// description: the TypeScript build words them once.
pub fn world_actions() -> Vec<ActionInfo> {
    WORLD
        .iter()
        .map(|(name, domain, label, flags, req)| ActionInfo {
            name: (*name).into(),
            domain: *domain,
            label: (*label).into(),
            description: format!("{label}."),
            traits: traits(*flags, req),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn library_sizes_and_uniqueness() {
        assert_eq!(land_action_names().len(), 53);
        let all: Vec<_> = land_action_names().into_iter().chain(world_action_names()).collect();
        let set: BTreeSet<_> = all.iter().collect();
        assert_eq!(set.len(), all.len(), "an action is listed twice");
        assert_eq!(world_action_names().len(), 139);
    }

    #[test]
    fn labels_follow_the_behaviour_manager() {
        assert_eq!(label_from_name("land.pullBackToFriends"), "Pull back to friends");
        assert_eq!(label_from_name("land.assault"), "Assault");
        let jam = world_actions().into_iter().find(|a| a.name == "ew.jam").unwrap();
        assert_eq!(jam.label, "Jam it");
        assert_eq!(jam.traits.required(), vec![Capability::Emitter, Capability::Jammer]);
    }
}
