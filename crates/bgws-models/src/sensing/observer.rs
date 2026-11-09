//! Human observers (§8.2): search with attention limits. A crew under fire,
//! suppressed or fatigued searches badly; a crew on overwatch with a sector
//! searches well; a crew that drives, fights or watches several things at
//! once splits its attention. The result is a multiplier on the search time
//! constant τ (see `acquire::Search::observer_tau_mult`) and a cap on how
//! many contacts it can keep track of. Suppression and fatigue losses follow
//! **[P31]** (−50–90%).

use bgws_core::{figure, Figure, Figures};
use serde::{Deserialize, Serialize};

/// What the observer is doing and how it is.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ObserverState {
    /// Suppression 0..1 (from `people::Suppression`).
    pub suppression: f64,
    /// Fatigue 0..1 (1 − effectiveness from `people::Fatigue`).
    pub fatigue: f64,
    /// Halted on overwatch with an assigned sector.
    pub overwatch: bool,
    /// Moving (driving, walking): attention on the route.
    pub moving: bool,
    /// Other tasks competing for attention (firing, reporting, navigating).
    pub other_tasks: u32,
    /// Contacts already being watched.
    pub contacts_held: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ObserverParams {
    pub suppression_loss: f64,
    pub fatigue_loss: f64,
    pub overwatch_factor: f64,
    pub moving_factor: f64,
    pub task_penalty: f64,
    pub capacity: f64,
}

impl ObserverParams {
    pub fn from_figures(f: &Figures) -> Self {
        ObserverParams {
            suppression_loss: f.get("sensing.observer.suppression_loss"),
            fatigue_loss: f.get("sensing.observer.fatigue_loss"),
            overwatch_factor: f.get("sensing.observer.overwatch_factor"),
            moving_factor: f.get("sensing.observer.moving_factor"),
            task_penalty: f.get("sensing.observer.task_penalty"),
            capacity: f.get("sensing.observer.capacity"),
        }
    }
}

/// The search-rate multiplier (≤ 1 slower, > 1 faster). τ multiplies by its inverse.
pub fn search_rate_factor(s: &ObserverState, p: &ObserverParams) -> f64 {
    let sup = 1.0 - p.suppression_loss * s.suppression.clamp(0.0, 1.0);
    let fat = 1.0 - p.fatigue_loss * s.fatigue.clamp(0.0, 1.0);
    let posture = if s.overwatch && !s.moving { 1.0 / p.overwatch_factor } else if s.moving { 1.0 / p.moving_factor } else { 1.0 };
    let tasks = 1.0 / (1.0 + p.task_penalty * s.other_tasks as f64);
    // Attention already spent on held contacts.
    let load = (1.0 - s.contacts_held as f64 / p.capacity).clamp(0.0, 1.0);
    (sup * fat * posture * tasks * load).max(0.0)
}

/// The τ multiplier for `acquire::Search` (infinite when no attention is left).
pub fn tau_multiplier(s: &ObserverState, p: &ObserverParams) -> f64 {
    let r = search_rate_factor(s, p);
    if r <= 0.0 {
        f64::INFINITY
    } else {
        1.0 / r
    }
}

/// Can the observer take on another contact?
pub fn has_capacity(s: &ObserverState, p: &ObserverParams) -> bool {
    (s.contacts_held as f64) < p.capacity
}

pub fn figures() -> Vec<Figure> {
    vec![
        figure!("sensing.observer.suppression_loss", 0.8, "fraction", 0.5, 0.9, "P31", Low, "search rate lost at full suppression (−50–90%)"),
        figure!("sensing.observer.fatigue_loss", 0.5, "fraction", 0.2, 0.8, "P31", Low, "search rate lost at full fatigue"),
        figure!("sensing.observer.overwatch_factor", 0.6, "fraction", 0.3, 1.0, "default", Default, "τ multiplier on overwatch with a sector"),
        figure!("sensing.observer.moving_factor", 2.0, "1", 1.0, 5.0, "default", Default, "τ multiplier while moving"),
        figure!("sensing.observer.task_penalty", 0.5, "1", 0.0, 2.0, "default", Default, "τ grows by this per competing task"),
        figure!("sensing.observer.capacity", 6.0, "contacts", 2.0, 12.0, "default", Default, "contacts one crew can keep track of"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::figs;

    #[test]
    fn suppressed_crews_search_badly() {
        let p = ObserverParams::from_figures(&figs());
        let calm = ObserverState { suppression: 0.0, fatigue: 0.0, overwatch: true, moving: false, other_tasks: 0, contacts_held: 0 };
        let under_fire = ObserverState { suppression: 1.0, overwatch: false, other_tasks: 1, ..calm };
        let f_calm = search_rate_factor(&calm, &p);
        let f_fire = search_rate_factor(&under_fire, &p);
        assert!(f_calm > 1.0 && f_fire < 0.2, "{f_calm} {f_fire}");
        let full = ObserverState { contacts_held: 6, ..calm };
        assert!(!has_capacity(&full, &p));
        assert!(tau_multiplier(&full, &p).is_infinite());
    }
}
