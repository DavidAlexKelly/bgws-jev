//! The after-action review (§16): read a game's event log (JSONL) and say
//! what happened and why. Every event carries the figures behind it (rule
//! 10), so the review can show the detection probability of a sighting, the
//! hit probability and penetration of a shot, or the source and confidence
//! of a decision.

use bgws_core::{LogEvent, Side, SimTime};
use std::collections::BTreeMap;
use std::io::BufRead;
use std::path::Path;

/// Read an event log written by `bgws play --events`.
pub fn read_log(path: &Path) -> anyhow::Result<Vec<LogEvent>> {
    let f = std::io::BufReader::new(std::fs::File::open(path)?);
    let mut out = Vec::new();
    for (i, line) in f.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        out.push(serde_json::from_str(&line).map_err(|e| anyhow::anyhow!("line {}: {e}", i + 1))?);
    }
    Ok(out)
}

fn side_word(s: Option<Side>) -> &'static str {
    s.map(|s| s.as_str()).unwrap_or("—")
}

/// The review as Markdown.
pub fn review(events: &[LogEvent]) -> String {
    let mut md = String::from("# After-action review\n\n");
    if let Some(start) = events.iter().find(|e| e.kind == "sim.start") {
        md.push_str(&format!("{}\n\n", start.text));
    }
    if let Some(over) = events.iter().rev().find(|e| e.kind == "sim.over") {
        md.push_str(&format!("**{}**\n\n", over.text));
    }
    // Activity by kind.
    let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
    for e in events {
        *kinds.entry(e.kind.split('.').next().unwrap_or("")).or_default() += 1;
    }
    md.push_str("## What the log holds\n\n| Domain | Events |\n|---|---|\n");
    for (k, n) in &kinds {
        md.push_str(&format!("| {k} | {n} |\n"));
    }
    // Losses.
    let destroyed: Vec<&LogEvent> = events.iter().filter(|e| e.kind == "unit.destroyed" || e.kind.ends_with(".sunk")).collect();
    md.push_str(&format!("\n## Losses ({})\n\n", destroyed.len()));
    for e in &destroyed {
        md.push_str(&format!("- {} — {} ({})\n", e.t, e.text, side_word(e.side)));
    }
    // Kill chains: first sighting of a target by its enemy, to the first shot at it.
    let mut first_seen: BTreeMap<u32, SimTime> = BTreeMap::new();
    let mut chains: Vec<(String, f64)> = Vec::new();
    for e in events {
        if e.kind == "sense.detect" {
            if let Some(t) = e.target {
                first_seen.entry(t.0).or_insert(e.t);
            }
        }
        if e.kind.starts_with("fire.") || e.kind == "fires.impact" || e.kind == "drones.strike" || e.kind == "munition.impact" {
            if let Some(t) = e.target {
                if let Some(seen) = first_seen.remove(&t.0) {
                    chains.push((format!("{}", t), (e.t - seen).secs()));
                }
            }
        }
    }
    if !chains.is_empty() {
        let mut d: Vec<f64> = chains.iter().map(|c| c.1).collect();
        d.sort_by(f64::total_cmp);
        md.push_str(&format!(
            "\n## Detect to first shot\n\n{} targets engaged after being seen: median {:.0} s, fastest {:.0} s, slowest {:.0} s.\n",
            d.len(),
            d[d.len() / 2],
            d[0],
            d[d.len() - 1]
        ));
    }
    // Fire.
    let shots: Vec<&LogEvent> = events.iter().filter(|e| matches!(e.kind.as_str(), "fire.miss" | "fire.hit" | "fire.kill")).collect();
    if !shots.is_empty() {
        md.push_str("\n## Direct fire by side\n\n| Side | Volleys | Hits | Knocked out | Mean Ph |\n|---|---|---|---|---|\n");
        for side in Side::PLAYERS {
            let mine: Vec<&&LogEvent> = shots.iter().filter(|e| e.side == Some(side)).collect();
            if mine.is_empty() {
                continue;
            }
            let hits: u64 = mine.iter().filter_map(|e| e.data["hits"].as_u64()).sum();
            let kills: u64 = mine.iter().filter_map(|e| e.data["kills"].as_u64()).sum();
            let ph: Vec<f64> = mine.iter().filter_map(|e| e.data["ph"].as_f64()).collect();
            md.push_str(&format!("| {side} | {} | {hits} | {kills} | {:.2} |\n", mine.len(), ph.iter().sum::<f64>() / ph.len().max(1) as f64));
        }
    }
    // Decisions.
    let answers: Vec<&LogEvent> = events.iter().filter(|e| e.kind == "decide.answer").collect();
    if !answers.is_empty() {
        let mut by: BTreeMap<(String, String), usize> = BTreeMap::new();
        for a in &answers {
            let src = a.data["source"]["kind"].as_str().unwrap_or("?").to_string();
            *by.entry((side_word(a.side).to_string(), src)).or_default() += 1;
        }
        md.push_str("\n## Decisions by who answered\n\n| Side | Source | Answers |\n|---|---|---|\n");
        for ((side, src), n) in by {
            md.push_str(&format!("| {side} | {src} | {n} |\n"));
        }
        let fallbacks: Vec<&&LogEvent> = answers.iter().filter(|a| a.data["source"]["kind"] == "fallback").collect();
        if !fallbacks.is_empty() {
            md.push_str("\nFallbacks (a decider could not answer):\n\n");
            for a in fallbacks.iter().take(20) {
                md.push_str(&format!("- {} {} — {}\n", a.t, a.text, a.data["source"]["reason"].as_str().unwrap_or("")));
            }
        }
    }
    // Timeline of the moments that mattered.
    md.push_str("\n## Timeline\n\n");
    for e in events.iter().filter(|e| {
        matches!(e.kind.as_str(), "unit.destroyed" | "people.morale" | "decide.plan" | "c2.plan_issued" | "c2.hq_lost" | "c2.handover" | "sim.over" | "lod.aggregate" | "lod.disaggregate" | "land.breach" | "logistics.culmination")
            || e.kind.ends_with(".sunk")
            || e.kind.ends_with(".missionKill")
    }) {
        md.push_str(&format!("- {} `{}` {}\n", e.t, e.kind, e.text));
    }
    md
}

/// Everything that happened to or by one force, with the figures behind
/// each event: the detection, link-budget and fire inspectors in one (§16).
pub fn why(events: &[LogEvent], who: &str) -> String {
    let id = events.iter().find(|e| e.text.contains(who) && e.entity.is_some()).and_then(|e| e.entity);
    let mut md = format!("# {who}\n\n");
    let Some(id) = id else {
        md.push_str("No events name it.\n");
        return md;
    };
    for e in events.iter().filter(|e| e.entity == Some(id) || e.target == Some(id)) {
        let role = if e.entity == Some(id) { "by" } else { "to" };
        md.push_str(&format!("- {} `{}` ({role}) {}", e.t, e.kind, e.text));
        if !e.data.is_null() {
            md.push_str(&format!("  \n  `{}`", e.data));
        }
        md.push('\n');
    }
    md
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_of_a_played_game() {
        let path = format!("{}/../../data/scenarios/skirmish-meeting-engagement-v1.json", env!("CARGO_MANIFEST_DIR"));
        let sc = bgws_scenario::v4::ScenarioV4::from_json(&std::fs::read_to_string(path).unwrap()).unwrap();
        let mut sim = bgws_sim::Sim::new(&sc, bgws_core::Seed(3), crate::default_figures()).unwrap();
        sim.run_with_rules(SimTime::from_secs(1800.0));
        let ev = sim.drain_events();
        let md = review(&ev);
        assert!(md.contains("## Losses") && md.contains("## Decisions by who answered"), "{md}");
        let unit = ev.iter().find(|e| e.kind == "fire.kill").map(|e| e.text.split(" fires").next().unwrap().to_string()).unwrap();
        assert!(why(&ev, &unit).contains("fire."));
    }
}
