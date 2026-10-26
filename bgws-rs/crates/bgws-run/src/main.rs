//! `bgws`: the command-line host (§3.3, §16).
//!
//! ```text
//! bgws scenarios                          list the shipped scenarios
//! bgws play <scenario> [--deciders rules|jev|claude|local] [--seed N] [--minutes M]
//!           [--record game.json] [--events events.jsonl] [--flat]
//! bgws replay <game.json>                 replay a record without calling any model
//! bgws branch <game.json> --at M [--deciders jev]   "what if": replay to minute M, then play on
//! bgws pack <dir> --name N --lat --lon      build a theatre pack of synthetic ground
//! bgws batch <scenario> [-n 100]          many seeded games on the rules, in parallel
//! bgws vignettes [--only id] [-n 200]     benchmark vignettes against their bands
//! bgws sensitivity <vignette>             Morris then Sobol over its figures; the research backlog
//! bgws aar <events.jsonl>                the after-action review
//! bgws why <events.jsonl> <unit>          what happened to or by a unit, and why
//! bgws figures [--markdown]               the figures registry report
//! bgws platform <id>                      a resolved catalogue record and its munitions
//! bgws convert <v3.json> <out.json>       convert a TypeScript v3 scenario to v4
//! bgws bench <scenario> [--seconds S]     time the simulation
//! ```
//! API keys come from the environment (`OPENROUTER_API_KEY`, `ANTHROPIC_API_KEY`), never from files.

use anyhow::{anyhow, bail, Result};
use bgws_analysis::report::VignetteSuite;
use bgws_analysis::CheckOptions;
use bgws_core::{Seed, SimTime};
use bgws_run::experiments;
use bgws_run::{load_scenario, DeciderChoice, Host, HostOptions};
use bgws_scenario::record::GameRecord;
use bgws_run::default_figures;
use bgws_sim::SimOptions;
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Parser)]
#[command(name = "bgws", version, about = "BGWS: a local, physics-first simulation with Jev and Claude as decision makers")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List the shipped scenarios.
    Scenarios,
    /// Play a scenario to the end (or a time limit).
    Play {
        scenario: String,
        #[arg(long, default_value = "scenario")]
        deciders: DeciderChoice,
        #[arg(long)]
        seed: Option<u64>,
        /// Stop after this many simulated minutes (default: the scenario's limit).
        #[arg(long)]
        minutes: Option<f64>,
        #[arg(long)]
        record: Option<PathBuf>,
        #[arg(long)]
        events: Option<PathBuf>,
        /// Flat open ground instead of the scenario's.
        #[arg(long)]
        flat: bool,
        #[arg(long, default_value = "http://127.0.0.1:8080")]
        local_url: String,
        #[arg(long, default_value = "local")]
        local_model: String,
        #[arg(long)]
        claude_model: Option<String>,
    },
    /// Replay a game record exactly, without calling any decider.
    Replay { record: PathBuf },
    /// "What if": replay a record to a moment, then play on with other deciders (§12).
    Branch {
        record: PathBuf,
        /// Simulated minutes to replay before branching.
        #[arg(long)]
        at: f64,
        #[arg(long, default_value = "rules")]
        deciders: DeciderChoice,
        /// Play this many minutes after the branch point (default: to the scenario's limit).
        #[arg(long)]
        minutes: Option<f64>,
        #[arg(long)]
        record_out: Option<PathBuf>,
    },
    /// Build a theatre pack (§14.2): synthetic ground for an area, tiled, for scenarios to name.
    Pack {
        out: PathBuf,
        #[arg(long)]
        name: String,
        #[arg(long)]
        lat: f64,
        #[arg(long)]
        lon: f64,
        /// Side of the square area, km.
        #[arg(long, default_value_t = 40.0)]
        size_km: f64,
        #[arg(long, default_value_t = 30.0)]
        cell_m: f64,
        #[arg(long, default_value_t = 1)]
        seed: u64,
    },
    /// Play many seeded games with the rules deciders.
    Batch {
        scenario: String,
        #[arg(short, default_value_t = 100)]
        n: usize,
        #[arg(long, default_value_t = 1)]
        seed: u64,
        #[arg(long)]
        minutes: Option<f64>,
        #[arg(long)]
        flat: bool,
        /// Write the batch summary as JSON here.
        #[arg(long)]
        json: Option<PathBuf>,
    },
    /// Check the benchmark vignettes against their bands.
    Vignettes {
        #[arg(long)]
        only: Option<String>,
        #[arg(short, default_value_t = 200)]
        n: usize,
        #[arg(long, default_value_t = 1)]
        seed: u64,
        #[arg(long)]
        json: Option<PathBuf>,
    },
    /// Sensitivity analysis of one vignette over its unsourced figures.
    Sensitivity {
        vignette: String,
        #[arg(long, default_value_t = 40)]
        replicates: usize,
        /// Only figures whose key starts with this.
        #[arg(long)]
        prefix: Option<String>,
        #[arg(long, default_value_t = 1)]
        seed: u64,
    },
    /// The after-action review of a game's event log (`play --events`).
    Aar { events: PathBuf },
    /// Everything that happened to or by one force, with the figures behind each event.
    Why { events: PathBuf, who: String },
    /// The figures registry as a report.
    Figures {
        #[arg(long)]
        markdown: bool,
    },
    /// Print a catalogue record, resolved (class defaults, parent, derivations).
    Platform { id: String },
    /// Convert a TypeScript v3 scenario to v4.
    Convert { input: PathBuf, output: PathBuf },
    /// Time the simulation on a scenario.
    Bench {
        scenario: String,
        #[arg(long, default_value_t = 600.0)]
        seconds: f64,
        #[arg(long)]
        flat: bool,
    },
}

fn data_dir() -> PathBuf {
    std::env::var_os("BGWS_DATA").map(PathBuf::from).unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data"))
}

/// A scenario by path, or by the name of a shipped one.
fn scenario_path(s: &str) -> PathBuf {
    let p = PathBuf::from(s);
    if p.exists() {
        return p;
    }
    let named = data_dir().join("scenarios").join(if s.ends_with(".json") { s.to_string() } else { format!("{s}.json") });
    if named.exists() {
        named
    } else {
        p
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Scenarios => {
            let mut names: Vec<_> = std::fs::read_dir(data_dir().join("scenarios"))?.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().map(|x| x == "json").unwrap_or(false)).collect();
            names.sort();
            for p in names {
                let sc = load_scenario(&p)?;
                println!("{:40} {:3} forces  {:?}  {} min  — {}", p.file_stem().unwrap().to_string_lossy(), sc.forces.len(), sc.scope.kind, sc.scope.duration_min, sc.name);
            }
        }
        Cmd::Play { scenario, deciders, seed, minutes, record, events, flat, local_url, local_model, claude_model } => {
            let sc = load_scenario(&scenario_path(&scenario))?;
            let seed = Seed(seed.unwrap_or(sc.seed.0));
            let opts = HostOptions { deciders, local_url, local_model, claude_model, sim: SimOptions { flat_ground: flat, ..Default::default() } };
            let mut host = Host::new(&sc, seed, default_figures(), &opts)?;
            if let Some(p) = &events {
                host.write_events_to(p)?;
            }
            let until = SimTime::from_secs(minutes.unwrap_or(sc.scope.duration_min) * 60.0);
            let started = Instant::now();
            let mut last_print = Instant::now();
            host.play_until(until, |sim| {
                if last_print.elapsed().as_secs_f64() > 2.0 {
                    let s = sim.strength();
                    eprintln!("  {}  blue {:.0}%  red {:.0}%", sim.now(), s.get(&bgws_core::Side::Blue).unwrap_or(&0.0) * 100.0, s.get(&bgws_core::Side::Red).unwrap_or(&0.0) * 100.0);
                    last_print = Instant::now();
                }
            })?;
            let report = host.report();
            println!("{}", serde_json::to_string_pretty(&report)?);
            eprintln!("played {} simulated in {:.1} s wall", report.ended_at, started.elapsed().as_secs_f64());
            if let Some(p) = record {
                let mut rec = host.record.clone();
                rec.fingerprint = Some(host.sim.fingerprint());
                rec.save(&p).map_err(|e| anyhow!("{e}"))?;
                eprintln!("record written to {}", p.display());
            }
        }
        Cmd::Replay { record } => {
            let rec = GameRecord::load(&record).map_err(|e| anyhow!("{e}"))?;
            let until = rec.outcome.as_ref().and_then(|o| o.get("at")).and_then(|v| v.as_i64()).map(SimTime).unwrap_or(SimTime::from_secs(rec.scenario.scope.duration_min * 60.0));
            let mut host = Host::replay(rec, default_figures())?;
            host.play_until(until, |_| {})?;
            host.verify_replay()?;
            println!("{}", serde_json::to_string_pretty(&host.report())?);
            eprintln!("replay matches the record");
        }
        Cmd::Branch { record, at, deciders, minutes, record_out } => {
            let rec = GameRecord::load(&record).map_err(|e| anyhow!("{e}"))?;
            let branch_at = SimTime::from_secs(at * 60.0);
            let mut host = Host::replay(rec.clone(), default_figures())?;
            host.play_until(branch_at, |_| {})?;
            eprintln!("replayed to {}; branching with {:?}", host.sim.now(), deciders);
            let opts = HostOptions { deciders, ..Default::default() };
            host.router = bgws_run::router_for(&rec.scenario, &default_figures(), &opts);
            host.stop_replay();
            let until = SimTime::from_secs(minutes.map(|m| at + m).unwrap_or(rec.scenario.scope.duration_min) * 60.0);
            host.play_until(until, |_| {})?;
            println!("{}", serde_json::to_string_pretty(&host.report())?);
            if let Some(p) = record_out {
                let mut r = host.record.clone();
                r.fingerprint = Some(host.sim.fingerprint());
                r.save(&p).map_err(|e| anyhow!("{e}"))?;
            }
        }
        Cmd::Pack { out, name, lat, lon, size_km, cell_m, seed } => {
            let f = default_figures();
            let n = ((size_km * 1000.0 / cell_m).ceil() as usize + 1).max(16);
            let mut p = bgws_env::SynthParams::new(seed, n, n, cell_m, &f);
            p.frame_origin = bgws_core::Geodetic::new(lat, lon, 0.0);
            let world = bgws_env::generate(&p, &f);
            let meta = bgws_env::TheatrePack::write(&out, &name, &world, 128, bgws_env::Codec::Lz4).map_err(|e| anyhow!("{e:?}"))?;
            eprintln!("theatre pack `{}`: {} tiles at {} m in {}", meta.name, meta.tiles.len(), meta.cell, out.display());
        }
        Cmd::Batch { scenario, n, seed, minutes, flat, json } => {
            let sc = load_scenario(&scenario_path(&scenario))?;
            let limit = SimTime::from_secs(minutes.unwrap_or(sc.scope.duration_min) * 60.0);
            let exp = experiments::scenario_run(&sc, limit, SimOptions { flat_ground: flat, ..Default::default() });
            let figures = default_figures();
            let started = Instant::now();
            let batch = bgws_analysis::batch::run_batch(&exp, &figures, bgws_analysis::batch::BatchConfig::new(n, Seed(seed)));
            let wall = started.elapsed().as_secs_f64();
            let mut outcomes: std::collections::BTreeMap<String, usize> = Default::default();
            for r in &batch.replicates {
                *outcomes.entry(r.output.over.clone().unwrap_or_else(|| "unfinished".into())).or_default() += 1;
            }
            let mut md = format!("# Batch: {} × {n}\n\n{wall:.1} s wall ({:.2} s per game)\n\n| Outcome | Games |\n|---|---|\n", sc.name, wall / n as f64);
            for (k, v) in &outcomes {
                md.push_str(&format!("| {k} | {v} |\n"));
            }
            for side in bgws_core::Side::PLAYERS {
                let m = bgws_analysis::report::strength_metric(side);
                let sample = batch.sample(&m);
                md.push('\n');
                md.push_str(&bgws_analysis::report::batch_markdown(&bgws_analysis::batch::BatchSummary::new(format!("{side} strength left"), batch.config, &sample, 0.95, 10)));
            }
            println!("{md}");
            if let Some(p) = json {
                std::fs::write(p, serde_json::to_string_pretty(&batch.replicates.iter().map(|r| &r.output).collect::<Vec<_>>())?)?;
            }
        }
        Cmd::Vignettes { only, n, seed, json } => {
            let figures = default_figures();
            let attack = load_scenario(&scenario_path("skirmish-advance-to-contact-v1")).ok();
            let results = experiments::run_vignettes(&figures, Seed(seed), &CheckOptions { level: 0.95, replicates: Some(n) }, only.as_deref(), attack.as_ref());
            let suite = VignetteSuite::new(results);
            println!("{}", suite.to_markdown());
            if let Some(p) = json {
                std::fs::write(p, serde_json::to_string_pretty(&suite)?)?;
            }
            if !suite.all_in_band() {
                bail!("some vignettes are out of band");
            }
        }
        Cmd::Sensitivity { vignette, replicates, prefix, seed } => {
            bgws_run::experiments::sensitivity(&vignette, replicates, prefix.as_deref(), Seed(seed))?;
        }
        Cmd::Aar { events } => println!("{}", bgws_run::aar::review(&bgws_run::aar::read_log(&events)?)),
        Cmd::Why { events, who } => println!("{}", bgws_run::aar::why(&bgws_run::aar::read_log(&events)?, &who)),
        Cmd::Figures { markdown } => {
            let f = default_figures();
            if markdown {
                println!("{}", f.report_markdown());
            } else {
                let defaults = f.iter().filter(|x| x.is_default()).count();
                println!("{} figures, {} sourced, {} [default]", f.len(), f.len() - defaults, defaults);
                println!("(--markdown for the full report)");
            }
        }
        Cmd::Platform { id } => {
            let cat = bgws_catalogue::Catalogue::builtin_with(&default_figures()).map_err(|e| anyhow!("{e:?}"))?;
            let p = cat.platform(&id).ok_or_else(|| anyhow!("no platform `{id}`"))?;
            println!("{}", serde_json::to_string_pretty(p)?);
            for m in &p.mounts {
                for mu in &m.munitions {
                    if let Some(x) = cat.munition(mu) {
                        println!("munition {}: {}", mu, serde_json::to_string(x)?);
                    }
                }
            }
        }
        Cmd::Convert { input, output } => {
            let sc = load_scenario(&input)?;
            std::fs::write(&output, serde_json::to_string_pretty(&sc)?)?;
            eprintln!("wrote {}", output.display());
        }
        Cmd::Bench { scenario, seconds, flat } => {
            let sc = load_scenario(&scenario_path(&scenario))?;
            let mut sim = bgws_sim::Sim::with_options(&sc, sc.seed, default_figures(), SimOptions { flat_ground: flat, ..Default::default() }).map_err(|e| anyhow!("{e}"))?;
            let started = Instant::now();
            sim.run_with_rules(SimTime::from_secs(seconds));
            let wall = started.elapsed().as_secs_f64();
            println!("{} entities, {:.0} simulated s in {:.3} s wall: ×{:.1} real time, {:.2} ms per simulated second", sc.forces.len(), sim.now().secs(), wall, sim.now().secs() / wall, wall * 1000.0 / sim.now().secs().max(1.0));
        }
    }
    Ok(())
}
