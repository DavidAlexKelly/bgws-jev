//! The decider layer the host calls (RUST_ENGINE.md §3.3, §10.2–10.6, §12):
//! decider and planner traits; the rules and fallback deciders; the Jev
//! client (OpenRouter Decisions API), Claude planners and decider (Anthropic
//! Messages API), a local OpenAI-compatible model, recorded answers for
//! replay, a person's seat, and the router that sends each question to the
//! right one.
//!
//! This crate never sees the world: it takes `bgws_core::Question`s, built by
//! the sim from what each agent believes, and returns `Answer`s. The clock
//! waits in simulated time, so a slow or failed call never changes the game,
//! only how fast it plays; every failure falls back to the template's choice
//! (or the heuristic planner) and the answer says why. Keys are read from the
//! environment by the host and never printed.

pub mod claude;
pub mod decider;
pub mod hq;
pub mod http;
pub mod jev;
pub mod key;
pub mod local;
pub mod recorded;
pub mod router;
pub mod rules;
pub mod schema_check;

pub use claude::{ClaudeConfig, ClaudeDecider, ClaudePlanner, MessagesClient, ToolChoice};
pub use decider::{reconcile, Decider, Planner};
pub use hq::{HeuristicPlanner, HqSituation, PlannerKind};
pub use jev::{JevClient, JevConfig};
pub use key::ApiKey;
pub use local::{LocalConfig, LocalDecider};
pub use recorded::{RecordedDecider, ReplayDivergence};
pub use router::{person_seat, PersonRequest, PersonSeat, Route, Router};
pub use rules::{FallbackDecider, RulesDecider};

use bgws_core::{figure, Figure, Figures};

/// Every figure this crate reads (rule 3).
pub fn figures() -> Vec<Figure> {
    vec![
        figure!("decide.jev.min_confidence", 0.25, "fraction", 0.0, 1.0, "default", Default, "Below this confidence Jev's choice is not taken; the template fallback stands (DECISION_FLOW_GRAPHS §3.3)."),
        figure!("decide.jev.timeout", 4.0, "s", 0.5, 120.0, "default", Default, "Wall-clock wait for one side's Jev request before the fallbacks decide; sim time waits, so it changes only pace."),
        figure!("decide.jev.cache_limit", 2000.0, "count", 1.0, 1.0e6, "default", Default, "Most remembered Jev answers; the oldest go first."),
        figure!("decide.jev.max_choices", 255.0, "count", 2.0, 255.0, "default", Default, "Jev's hard limit on options in one choice question."),
        figure!("decide.claude.max_tokens", 16000.0, "count", 1024.0, 128000.0, "default", Default, "Output cap for one planning request."),
        figure!("decide.claude.retries", 3.0, "count", 0.0, 10.0, "default", Default, "Retries on 408/409/429/5xx and connection failures."),
        figure!("decide.claude.backoff_base", 0.5, "s", 0.001, 30.0, "default", Default, "First retry delay; doubles each retry."),
        figure!("decide.claude.backoff_max", 8.0, "s", 0.001, 120.0, "default", Default, "Longest retry delay, including a server's retry-after."),
        figure!("decide.claude.timeout", 120.0, "s", 1.0, 600.0, "default", Default, "Wall-clock wait for one planning request."),
        figure!("decide.claude.price_input", 2.0, "USD/Mtok", 0.0, 100.0, "default", Default, "Input price per million tokens (claude-sonnet-5-5 list price, October 2026)."),
        figure!("decide.claude.price_output", 10.0, "USD/Mtok", 0.0, 500.0, "default", Default, "Output price per million tokens (claude-sonnet-5-5 list price, October 2026)."),
        figure!("decide.local.timeout", 10.0, "s", 0.5, 600.0, "default", Default, "Wait for one local-model answer."),
        figure!("decide.local.min_confidence", 0.25, "fraction", 0.0, 1.0, "default", Default, "Below this (from logprobs) the local model's choice is not taken."),
        figure!("decide.local.top_logprobs", 10.0, "count", 1.0, 20.0, "default", Default, "Alternatives asked for at each token, to read option probabilities."),
        figure!("decide.heuristic.culmination_loss", 0.2, "fraction", 0.1, 0.4, "P14", Medium, "An HQ halts its attack once its units have lost this share of their vehicles (attacks halt at about 20% casualties)."),
        figure!("decide.heuristic.enemy_radius", 2500.0, "m", 500.0, 10000.0, "default", Default, "Known land tracks within this of the objective count as its defence."),
        figure!("decide.heuristic.attack_ratio", 1.2, "ratio", 0.5, 4.0, "default", Default, "Own to known enemy strength at which the heuristic attacks."),
        figure!("decide.heuristic.strength_per_track", 8.0, "strength", 1.0, 50.0, "default", Default, "Strength assumed for a known track when the HQ has no units to compare."),
        figure!("decide.heuristic.frontage", 300.0, "m", 50.0, 2000.0, "default", Default, "Spacing between lead units across the objective."),
        figure!("decide.heuristic.reserve_back", 1800.0, "m", 200.0, 10000.0, "default", Default, "Where the reserve waits, behind the objective on the axis."),
        figure!("decide.heuristic.engineer_back", 1200.0, "m", 100.0, 10000.0, "default", Default, "Where engineers follow the lead, behind the objective."),
        figure!("decide.heuristic.post_back", 2500.0, "m", 200.0, 20000.0, "default", Default, "Where a command post follows the attack to."),
        figure!("decide.heuristic.post_follow_beyond", 4000.0, "m", 500.0, 50000.0, "default", Default, "A command post further than this from the objective follows the attack."),
        figure!("decide.heuristic.fires_move_up", 0.7, "fraction", 0.1, 1.0, "default", Default, "Fire units beyond this share of their reach move up."),
        figure!("decide.heuristic.fires_standoff", 0.5, "fraction", 0.1, 1.0, "default", Default, "Where fire units move up to, as a share of their reach from the objective."),
        figure!("decide.heuristic.enemy_close", 3000.0, "m", 200.0, 20000.0, "default", Default, "A lead unit with a known enemy this close bounds instead of moving tactically."),
        figure!("decide.heuristic.culmination_min_distance", 300.0, "m", 0.0, 5000.0, "default", Default, "An attack on its objective does not culminate; only one still short of it."),
        figure!("decide.heuristic.min_units_for_reserve", 3.0, "count", 2.0, 10.0, "default", Default, "Manoeuvre units needed before one is held back as the reserve."),
    ]
}

/// This crate's figures at their declared values.
pub fn default_figures() -> Figures {
    // The list is checked by a test, so this cannot fail in a released build.
    Figures::from_list(figures()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn figures_register_and_are_all_used() {
        let f = Figures::from_list(figures()).unwrap();
        let _ = jev::JevConfig::from_figures(&f);
        let _ = claude::ClaudeConfig::from_figures(&f);
        let _ = local::LocalConfig::from_figures(&f, "http://x", "m");
        let _ = hq::HeuristicParams::from_figures(&f);
        assert!(f.unused().is_empty(), "{:?}", f.unused());
    }
}
