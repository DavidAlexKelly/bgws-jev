//! `bgws`: the command-line host (§3.3, §16).
//!
//! ```text
//! bgws scenarios                          list the shipped scenarios
//! bgws play <scenario> [--deciders rules|jev|claude|local] [--seed N] [--minutes M]
//!           [--record game.json] [--events events.jsonl] [--flat]
//! bgws replay <game.json>                 replay a record without calling any model
//! bgws branch <game.json> --at M [--deciders jev]   "what if": replay to minute M, then play on
//! bgws pack <dir> --name N --lat --lon [--dem dem.tif]   build a theatre pack (generated ground)
//! bgws terrain <dir> --bbox W,S,E,N [--name N] [--cell-m 30]   build a theatre pack from real ground
//! bgws batch <scenario> [-n 100]          many seeded games on the rules, in parallel
//! bgws vignettes [--only id] [-n 200] [--store dir]   benchmark vignettes; regression against a baseline
//! bgws sensitivity <vignette>             Morris then Sobol over its figures; the research backlog
//! bgws aar <events.jsonl>                the after-action review
//! bgws events <events.duckdb> [--sql Q]  SQL over a game's events (needs --features duckdb)
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
use bgws_run::default_figures;
use bgws_run::experiments;
use bgws_run::{load_scenario, DeciderChoice, Host, HostOptions};
use bgws_scenario::record::GameRecord;
use bgws_sim::SimOptions;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
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
        /// Play in real time: the clock follows the wall clock (at this many
        /// simulated seconds per second; 1 without a value) and decisions do
        /// not stop it. Answers apply when they come back.
        #[arg(long, num_args = 0..=1, default_missing_value = "1")]
        realtime: Option<f64>,
        /// Publish the game over DIS (v7 Entity State PDUs) to this address,
        /// `host:port` (a broadcast address is allowed); implies --realtime.
        #[arg(long)]
        dis: Option<String>,
        /// The DIS exercise ID (else the scenario's `interop.exercise`, else 1).
        #[arg(long)]
        dis_exercise: Option<u8>,
        /// Also take in other simulations' DIS on this address (`host:port`):
        /// their entities join as remote entities, their detonations on ours
        /// are judged here. Without it the game only publishes.
        #[arg(long)]
        dis_listen: Option<String>,
        /// Join this IPv4 multicast group to listen.
        #[arg(long)]
        dis_group: Option<std::net::Ipv4Addr>,
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
    /// Build a theatre pack from real ground for a box (§14.2): elevation (Mapterhorn), land cover,
    /// water, towns and roads (a Protomaps OpenStreetMap build), and the viewer's basemap, read by
    /// HTTP range from planet PMTiles and cached. Scenarios name the folder as `theatre.pack`.
    Terrain {
        out: PathBuf,
        /// west,south,east,north in degrees.
        #[arg(long, allow_hyphen_values = true)]
        bbox: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long, default_value_t = 30.0)]
        cell_m: f64,
        /// Protomaps archive (URL or path); the latest daily build by default.
        #[arg(long)]
        planet: Option<String>,
        /// Elevation archive (terrarium PMTiles, URL or path).
        #[arg(long, default_value = bgws_geodata::build::ELEVATION_URL)]
        elevation: String,
        /// Bare ground under woods and towns: the elevation is a surface model
        /// (trees and roofs in it), filtered over this window, metres; 0 keeps it.
        #[arg(long, default_value_t = 150.0)]
        ground_filter_m: f64,
        /// Road vertices closer than this join, metres.
        #[arg(long, default_value_t = 3.0)]
        snap_m: f64,
        /// Highest basemap zoom kept for the viewer.
        #[arg(long, default_value_t = 14)]
        basemap_max_zoom: u8,
        /// Where fetched tiles are kept ($BGWS_CACHE, else the platform cache folder).
        #[arg(long)]
        cache: Option<PathBuf>,
    },
    /// Work out (again) a theatre pack's terrain appreciation: dead ground, mobility
    /// corridors, chokepoints, crossings and observation points, saved with the pack.
    /// `bgws terrain` does this as it builds; a pack without one gets it the first
    /// time a game opens on it.
    Analyse { pack: PathBuf },
    /// Compare a scenario's ground with a partner simulation's elevation (a
    /// GeoTIFF on a latitude/longitude grid) before a federation runs:
    /// height errors on a grid, and sight lines judged on both grounds.
    Correlate {
        scenario: String,
        partner: PathBuf,
        #[arg(long, default_value_t = 250.0)]
        spacing: f64,
        /// Height difference worth listing, m.
        #[arg(long, default_value_t = 2.0)]
        tolerance: f64,
        #[arg(long, default_value_t = 2000)]
        sight_lines: usize,
        /// The partner's heights are above the WGS84 ellipsoid (not mean sea level).
        #[arg(long)]
        ellipsoidal: bool,
        /// Write the full report (every mismatch and disagreement) here as JSON.
        #[arg(long)]
        json: Option<PathBuf>,
    },
    /// Write a scenario's ground as a GeoTIFF (latitude/longitude, heights
    /// above mean sea level) for a partner simulation to build its terrain from.
    ExportGround {
        scenario: String,
        out: PathBuf,
        #[arg(long, default_value_t = 30.0)]
        cell_m: f64,
    },
    /// The decision lab: collect units' decisions from games, play out every
    /// option, grade a decider against the results and your labels.
    Lab {
        #[command(subcommand)]
        cmd: LabCmd,
        /// The lab folder ($BGWS_LAB, else ./lab).
        #[arg(long, global = true)]
        lab: Option<PathBuf>,
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
        /// A GeoTIFF DEM (lat/lon grid, e.g. Copernicus GLO-30) for real elevation; land cover stays synthetic.
        #[arg(long)]
        dem: Option<PathBuf>,
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
        /// Compare each distribution with the stored baseline here (§15.8): out of band
        /// or a significant shift fails. New vignettes are stored as baselines.
        #[arg(long)]
        store: Option<PathBuf>,
        /// Accept this run as the new baseline, with the reason (needs --store).
        #[arg(long)]
        accept: Option<String>,
        /// Vignettes on the calibration backlog (README): run and reported,
        /// but being out of band does not fail the run.
        #[arg(long, value_delimiter = ',')]
        backlog: Vec<String>,
        /// Also write the workbench page (charts of each vignette against its band and baseline).
        #[arg(long)]
        html: Option<PathBuf>,
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
        /// Also write the workbench page (figures ranked by influence).
        #[arg(long)]
        html: Option<PathBuf>,
    },
    /// The Behaviour Manager page: every template's decision graphs and the action library.
    Behaviour {
        #[arg(long, default_value = "behaviour.html")]
        html: PathBuf,
    },
    /// Run a vignette as shipped and with figures changed, and compare (§16 workbench).
    Compare {
        vignette: String,
        /// Figures for the variant: key=value, comma-separated.
        #[arg(long, value_delimiter = ',', required = true)]
        set: Vec<String>,
        #[arg(short, default_value_t = 100)]
        n: usize,
        #[arg(long, default_value_t = 1)]
        seed: u64,
        /// Also write the workbench page with the two side by side.
        #[arg(long)]
        html: Option<PathBuf>,
    },
    /// SQL over a game's event log (table `events`); with no query, events by kind.
    Events {
        log: PathBuf,
        #[arg(long)]
        sql: Option<String>,
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
    bgws_run::data_dir()
}

/// The scenarios the sim-played vignettes run on, from the shipped presets.
fn vignette_scenarios() -> experiments::VignetteScenarios {
    let load = |name: &str| load_scenario(&scenario_path(name)).ok();
    experiments::VignetteScenarios {
        attack: load("skirmish-advance-to-contact-v1"),
        drone_line: load("m4-drone-line"),
        counter_battery: load("m2-mech-company-hasty-attack").and_then(|b| experiments::counter_battery_scene(&b).ok()),
    }
}

/// A scenario by path, or by the name of a shipped one.
#[derive(clap::Subcommand)]
enum LabCmd {
    /// Play a scenario (one game per seed) and keep every unit decision.
    Collect {
        scenario: String,
        /// Seeds: `4` (one), `1-8` (a range) or `1,3,9`.
        #[arg(long, default_value = "1")]
        seeds: String,
        #[arg(long, default_value = "rules")]
        deciders: DeciderChoice,
        /// How long each game runs; the scenario's length by default.
        #[arg(long)]
        minutes: Option<f64>,
    },
    /// Keep a saved game's unit decisions (`--record` from `play`, or the viewer's Save).
    Import { record: PathBuf },
    /// Play out every option of moments: replay to the decision, force the
    /// option, play on with fresh dice, score from the unit's side.
    Playout {
        /// One moment; else every moment not played out yet (or all, with --again).
        #[arg(long)]
        moment: Option<String>,
        #[arg(long, default_value_t = 12)]
        runs: usize,
        #[arg(long, default_value_t = 20.0)]
        horizon_min: f64,
        /// At most this many moments.
        #[arg(long)]
        limit: Option<usize>,
        /// Only this decision point (D1…).
        #[arg(long)]
        point: Option<String>,
        #[arg(long)]
        again: bool,
    },
    /// Ask a decider every moment's question afresh and keep its answers.
    Grade {
        #[arg(long, default_value = "jev")]
        deciders: DeciderChoice,
        /// The model (Jev or Claude), else the configured one.
        #[arg(long)]
        model: Option<String>,
        /// Jev only: give the situation in words as well as the fields.
        #[arg(long)]
        prose: bool,
        /// Only moments that have been played out or labelled.
        #[arg(long)]
        judged_only: bool,
        #[arg(long, default_value = "http://127.0.0.1:8080")]
        local_url: String,
        #[arg(long, default_value = "local")]
        local_model: String,
    },
    /// Test a decider on the decision cases (data/cases and the lab's own):
    /// hand-written situations with the answers expected.
    Test {
        #[arg(long, default_value = "jev")]
        deciders: DeciderChoice,
        #[arg(long)]
        model: Option<String>,
        /// Jev only: give the situation in words as well as the fields.
        #[arg(long)]
        prose: bool,
        /// Only cases whose id contains this.
        #[arg(long)]
        case: Option<String>,
        /// Only this decision point (D1…).
        #[arg(long)]
        point: Option<String>,
        /// Check that every case builds (its options exist) and show the template fallback's score; ask no one.
        #[arg(long)]
        check: bool,
    },
    /// The engagement suite: play its scenarios with the decider on one side
    /// against the rules, paired with a rules-against-rules game on the same
    /// seed, and report what the decider changes (wins, losses, survival).
    Suite {
        #[arg(long, default_value = "jev")]
        deciders: DeciderChoice,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        prose: bool,
        /// Seeds: `1-5`, `1,3,9`.
        #[arg(long, default_value = "1-4")]
        seeds: String,
        /// Only these scenarios (comma-separated names); else the suite's.
        #[arg(long)]
        scenarios: Option<String>,
        /// Games played at once (each waits on the decider's answers).
        #[arg(long, default_value_t = 4)]
        parallel: usize,
        /// Also the held-out scenarios (kept back from tuning).
        #[arg(long)]
        all: bool,
        /// Keep the decider's N worst games in the lab, to play out their decisions.
        #[arg(long, default_value_t = 0)]
        keep_worst: usize,
        /// Test the HQ planner instead of a decider: the heuristic planner with
        /// fire and movement on the side under test, without it everywhere else
        /// (the rules decide for every unit).
        #[arg(long)]
        planner_ab: bool,
    },
    /// Print the wording Jev is asked in (data/jev/wording.json, else the engine's own).
    Wording,
    /// Write the generated test scenarios (data/scenarios/gen-*.json): attacks,
    /// defences, a meeting engagement, an ambush, a withdrawal, and on a pack a
    /// river crossing and a fight into a town.
    Generate {
        /// Only this one (its name).
        #[arg(long)]
        only: Option<String>,
        /// Generate these specs instead (a JSON list of scengen specs).
        #[arg(long)]
        spec: Option<PathBuf>,
    },
    /// Tune Jev's wording: a larger model proposes rewrites from Jev's
    /// failures; the cases and the suite judge them; the held-out cases and
    /// scenarios check the result. Nothing changes until `--accept`.
    Tune {
        #[arg(long, default_value_t = 3)]
        rounds: usize,
        /// Rewrites asked for each round.
        #[arg(long, default_value_t = 4)]
        candidates: usize,
        /// The proposer (an OpenRouter chat model).
        #[arg(long, default_value = bgws_run::tune::DEFAULT_PROPOSER)]
        proposer: String,
        /// Jev's model, else the configured one.
        #[arg(long)]
        model: Option<String>,
        /// Judge on the cases only (no played games).
        #[arg(long)]
        no_suite: bool,
        #[arg(long, default_value = "1-4")]
        suite_seeds: String,
        /// Take a run's tuned wording: write it to data/jev/wording.json.
        #[arg(long)]
        accept: Option<String>,
    },
    /// Make a decision case from a lab moment (your label, else its play-outs).
    Case {
        moment: String,
        #[arg(long, default_value = "")]
        why: String,
    },
    /// How each decider does: near-best against the play-outs, agreement with your labels, calibration.
    Report {
        /// Every decision point, not only the totals.
        #[arg(long)]
        points: bool,
        #[arg(long)]
        json: bool,
    },
}

fn parse_seeds(s: &str) -> Result<Vec<u64>> {
    let mut v = Vec::new();
    for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        match part.split_once('-') {
            Some((a, b)) => v.extend(a.parse::<u64>()?..=b.parse::<u64>()?),
            None => v.push(part.parse()?),
        }
    }
    Ok(v)
}

fn scenario_path(s: &str) -> PathBuf {
    bgws_run::scenario_file(s)
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
        Cmd::Play { scenario, deciders, seed, minutes, record, events, flat, local_url, local_model, claude_model, realtime, dis, dis_exercise, dis_listen, dis_group } => {
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
            let mut progress = |sim: &bgws_sim::Sim| {
                if last_print.elapsed().as_secs_f64() > 2.0 {
                    let s = sim.strength();
                    eprintln!("  {}  blue {:.0}%  red {:.0}%", sim.now(), s.get(&bgws_core::Side::Blue).unwrap_or(&0.0) * 100.0, s.get(&bgws_core::Side::Red).unwrap_or(&0.0) * 100.0);
                    last_print = Instant::now();
                }
            };
            let pace = realtime.or(dis.as_ref().map(|_| 1.0));
            match pace {
                None => {
                    host.play_until(until, progress)?;
                }
                Some(speed) => {
                    eprintln!("playing in real time at {speed}x");
                    match &dis {
                        Some(addr) => {
                            let mut spec = sc.interop.clone().unwrap_or_default();
                            if let Some(why) = spec.ground_refusal(&sc.theatre, flat) {
                                bail!("not joining a DIS exercise: {why}");
                            }
                            if let Some(x) = dis_exercise {
                                spec.exercise = x;
                            }
                            eprintln!("DIS v7 to {addr} (exercise {}, site {}, application {}){}", spec.exercise, spec.site, spec.application, dis_listen.as_ref().map(|l| format!(", listening on {l}")).unwrap_or_default());
                            let link = bgws_interop::link::DisLink::new(addr, dis_listen.as_deref(), dis_group, spec, bgws_interop::dr::DrSettings::default())?;
                            let mut adapter = bgws_run::DisLinkAdapter::new(link, &mut progress);
                            host.play_realtime(until, Some(speed), &mut adapter)?;
                            for n in &adapter.notes {
                                eprintln!("DIS: {n}");
                            }
                            if let Some(e) = adapter.errors.first() {
                                eprintln!("DIS: {} sends failed (first: {e})", adapter.errors.len());
                            }
                            eprintln!("DIS: {} PDUs sent, {} inputs taken in", adapter.link.publisher.sent, adapter.link.received);
                        }
                        None => {
                            host.play_realtime(until, Some(speed), &mut bgws_run::OnTick(&mut progress))?;
                        }
                    }
                }
            }
            let report = host.report();
            println!("{}", serde_json::to_string_pretty(&report)?);
            eprintln!("played {} simulated in {:.1} s wall", report.ended_at, started.elapsed().as_secs_f64());
            if let Some(p) = record {
                host.finished_record().save(&p).map_err(|e| anyhow!("{e}"))?;
                eprintln!("record written to {}", p.display());
            }
        }
        Cmd::Replay { record } => {
            let rec = GameRecord::load(&record).map_err(|e| anyhow!("{e}"))?;
            let until = rec.ends_at();
            let realtime = rec.run.realtime;
            let mut host = Host::replay(rec, default_figures())?;
            if realtime {
                host.play_realtime(until, None, &mut bgws_run::OnTick(|_| {}))?;
            } else {
                host.play_until(until, |_| {})?;
            }
            host.verify_replay()?;
            println!("{}", serde_json::to_string_pretty(&host.report())?);
            eprintln!("replay matches the record");
        }
        Cmd::Branch { record, at, deciders, minutes, record_out } => {
            let rec = GameRecord::load(&record).map_err(|e| anyhow!("{e}"))?;
            let branch_at = SimTime::from_secs(at * 60.0);
            let opts = HostOptions { deciders, ..Default::default() };
            let router = bgws_run::router_for(&rec.scenario, &default_figures(), &opts);
            let mut host = Host::branch(rec.clone(), default_figures(), branch_at, router)?;
            eprintln!("replayed to {}; branching with {:?}", host.sim.now(), deciders);
            let until = SimTime::from_secs(minutes.map(|m| at + m).unwrap_or(rec.scenario.scope.duration_min) * 60.0);
            host.play_until(until, |_| {})?;
            println!("{}", serde_json::to_string_pretty(&host.report())?);
            if let Some(p) = record_out {
                host.finished_record().save(&p).map_err(|e| anyhow!("{e}"))?;
            }
        }
        Cmd::Terrain { out, bbox, name, cell_m, planet, elevation, ground_filter_m, snap_m, basemap_max_zoom, cache } => {
            let bbox = bgws_geodata::mercator::BBox::parse(&bbox).map_err(|e| anyhow!(e))?;
            let mut spec = bgws_geodata::build::BuildSpec::new(bbox);
            spec.cell_m = cell_m;
            spec.planet = planet;
            spec.elevation = elevation;
            spec.ground_filter_m = ground_filter_m;
            spec.basemap_max_zoom = basemap_max_zoom;
            if cache.is_some() {
                spec.cache = cache;
            }
            let name = name.unwrap_or_else(|| out.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "theatre".into()));
            let started = Instant::now();
            let last = std::sync::Mutex::new((String::new(), 0u32));
            let progress = |stage: &str, f: f64| {
                // A line per stage and every tenth of it.
                let step = (f * 10.0).floor() as u32;
                if let Ok(mut l) = last.lock() {
                    if l.0 != stage || step > l.1 {
                        *l = (stage.to_string(), step);
                        eprintln!("[{:6.1} s] {stage} {:>3.0}%", started.elapsed().as_secs_f64(), f * 100.0);
                    }
                }
            };
            let meta = bgws_geodata::pack::build_pack(&out, &name, &spec, snap_m, &progress).map_err(|e| anyhow!("{e}"))?;
            let a = bgws_run::host::analyse_pack(&out, &progress)?;
            eprintln!("terrain analysis: {} key terrain points", a.key.len());
            println!("{}", serde_json::to_string_pretty(&meta.sources).unwrap_or_default());
            eprintln!("theatre pack `{}` in {} ({} terrain tiles, {:.0} s)", meta.name, out.display(), meta.tiles.len(), started.elapsed().as_secs_f64());
        }
        Cmd::Correlate { scenario, partner, spacing, tolerance, sight_lines, ellipsoidal, json } => {
            let sc = load_scenario(&scenario_path(&scenario))?;
            let sim = bgws_run::correlate::ground_sim(&sc)?;
            let dem = bgws_run::geotiff::Dem::read(&partner)?;
            let geoid = bgws_interop::geoid::Geoid::egm96();
            let ground = &sim.ctx.ground;
            let ours = |e: f64, n: f64| ground.height(e, n);
            let theirs = |lat: f64, lon: f64| dem.height(lat, lon).map(|h| if ellipsoidal { geoid.to_msl(lat, lon, h) } else { h });
            let area = bgws_run::correlate::ground_area(ground);
            let r = bgws_run::correlate::correlate(&sim.ctx.frame, area, spacing, tolerance, sight_lines, &ours, &theirs, 1);
            println!("samples {} ({} not covered by the partner)", r.samples, r.uncovered);
            println!("height, ours − theirs: mean {:+.2} m, rms {:.2} m, 95th percentile |{:.2}| m, worst |{:.2}| m", r.mean_error_m, r.rms_error_m, r.p95_abs_error_m, r.max_abs_error_m);
            println!("beyond {:.1} m: {} points", r.tolerance_m, r.beyond_tolerance.len());
            println!("sight lines: {} of {} judged differently ({:.1}%)", r.sight_disagreements.len(), r.sight_lines, 100.0 * r.sight_disagreement_share());
            if let Some(p) = json {
                std::fs::write(&p, serde_json::to_string_pretty(&r)?)?;
                eprintln!("report in {}", p.display());
            }
        }
        Cmd::ExportGround { scenario, out, cell_m } => {
            let sc = load_scenario(&scenario_path(&scenario))?;
            let sim = bgws_run::correlate::ground_sim(&sc)?;
            let (w, h) = bgws_run::correlate::export_ground(&sim, &out, cell_m)?;
            eprintln!("{w} × {h} heights in {}", out.display());
        }
        Cmd::Lab { cmd, lab } => {
            let lab = bgws_run::lab::Lab::open(&lab.unwrap_or_else(bgws_run::lab::lab_dir))?;
            match cmd {
                LabCmd::Collect { scenario, seeds, deciders, minutes } => {
                    let sc = load_scenario(&scenario_path(&scenario))?;
                    for seed in parse_seeds(&seeds)? {
                        let t0 = Instant::now();
                        let ids = lab.collect(&sc, seed, deciders, minutes.unwrap_or(sc.scope.duration_min), default_figures())?;
                        eprintln!("seed {seed}: {} unit decisions kept ({:.0} s)", ids.len(), t0.elapsed().as_secs_f64());
                    }
                }
                LabCmd::Import { record } => {
                    let r = bgws_scenario::record::GameRecord::load(&record).map_err(|e| anyhow!("{}: {e}", record.display()))?;
                    let ids = lab.import(r, default_figures())?;
                    eprintln!("{} unit decisions kept", ids.len());
                }
                LabCmd::Playout { moment, runs, horizon_min, limit, point, again } => {
                    let ids: Vec<String> = match moment {
                        Some(m) => vec![m],
                        None => lab
                            .moments()?
                            .into_iter()
                            .filter(|m| again || m.playouts.is_none())
                            .filter(|m| point.as_ref().is_none_or(|p| &m.trigger == p))
                            .map(|m| m.id)
                            .take(limit.unwrap_or(usize::MAX))
                            .collect(),
                    };
                    let f = default_figures();
                    for (k, id) in ids.iter().enumerate() {
                        let t0 = Instant::now();
                        let m = lab.playout(id, runs, horizon_min * 60.0, bgws_run::lab::Weights::default(), &f)?;
                        let p = m.playouts.as_ref().unwrap();
                        let line: Vec<String> = p.options.iter().map(|o| format!("{} {:+.3}±{:.3}", o.option, o.mean, o.se)).collect();
                        eprintln!("[{}/{}] {} {} {}: best {} ({}) [{:.0} s]", k + 1, ids.len(), m.agent, m.trigger, m.title, p.best, line.join(", "), t0.elapsed().as_secs_f64());
                    }
                }
                LabCmd::Grade { deciders, model, prose, judged_only, local_url, local_model } => {
                    let (d, name) = bgws_run::lab::decider_for(deciders, &default_figures(), model, prose, (&local_url, &local_model))?;
                    let n = lab.grade(d, &name, |m| !judged_only || m.playouts.is_some() || m.label.is_some())?;
                    eprintln!("{n} moments graded as {name}");
                }
                LabCmd::Test { deciders, model, prose, case, point, check } => {
                    let (bases, cases) = bgws_run::cases::load(&bgws_run::cases::case_dirs(&lab.dir))?;
                    let cases: Vec<_> = cases.into_iter().filter(|c| case.as_ref().is_none_or(|k| c.id.contains(k.as_str())) && point.as_ref().is_none_or(|p| &c.point == p)).collect();
                    let mut b = bgws_run::cases::Builder::new(bases, &default_figures())?;
                    let items = bgws_run::cases::build_all(&mut b, &cases)?;
                    let fb = bgws_run::cases::judge(&cases, &items, &bgws_run::cases::fallback_answers(&items), "template fallback");
                    if check {
                        eprintln!("{} cases, {} questions: every one builds", cases.len(), items.len());
                        print!("{}", fb.table());
                    } else {
                        let (d, name) = bgws_run::cases::decider(deciders, &default_figures(), model, prose)?;
                        let t0 = Instant::now();
                        let run = bgws_run::cases::run(&cases, &items, d, &name)?;
                        let p = bgws_run::cases::save_run(&lab.dir, &run)?;
                        bgws_run::cases::save_run(&lab.dir, &fb)?;
                        print!("{}", run.table());
                        eprintln!("\n{} questions in {:.0} s, ${:.4}; kept in {}. The template fallback scores {} pass, {} wrong.", items.len(), t0.elapsed().as_secs_f64(), run.cost_usd, p.display(), fb.pass, fb.wrong);
                    }
                }
                LabCmd::Suite { deciders, model, prose, seeds, scenarios, parallel, all, keep_worst, planner_ab } => {
                    let entries: Vec<bgws_run::suite::Entry> = match scenarios {
                        Some(list) => list.split(',').map(|s| bgws_run::suite::Entry { scenario: s.trim().to_string(), minutes: None, note: String::new(), held_out: false }).collect(),
                        None => bgws_run::suite::load_entries()?.into_iter().filter(|e| all || !e.held_out).collect(),
                    };
                    let opts = bgws_run::suite::Options { keep_worst: (keep_worst > 0).then(|| (lab.dir.clone(), keep_worst)) };
                    let r = if planner_ab {
                        let f = default_figures();
                        let mut old = f.clone();
                        old.set("decide.heuristic.fire_and_movement", 0.0).map_err(|e| anyhow!("{e}"))?;
                        let test: std::sync::Arc<dyn bgws_decide::Planner> = std::sync::Arc::new(bgws_decide::HeuristicPlanner::from_figures(&f));
                        let base: std::sync::Arc<dyn bgws_decide::Planner> = std::sync::Arc::new(bgws_decide::HeuristicPlanner::from_figures(&old));
                        bgws_run::suite::run_planners(&entries, &parse_seeds(&seeds)?, test, base, "heuristic planner: fire and movement", &f, parallel, &opts, &|l| eprintln!("{l}"))?
                    } else {
                        let (d, name) = bgws_run::lab::decider_for(deciders, &default_figures(), model, prose, ("http://127.0.0.1:8080", "local"))?;
                        bgws_run::suite::run(&entries, &parse_seeds(&seeds)?, d, &name, &default_figures(), parallel, &opts, &|l| eprintln!("{l}"))?
                    };
                    let p = bgws_run::suite::save(&lab.dir, &r)?;
                    print!("{}", r.table());
                    eprintln!("kept in {}", p.display());
                }
                LabCmd::Tune { rounds, candidates, proposer, model, no_suite, suite_seeds, accept } => match accept {
                    Some(id) => {
                        let (p, d) = bgws_run::tune::accept(&lab.dir, &id)?;
                        println!("{d}");
                        eprintln!("wrote {}: Jev is asked in the tuned wording from now on (commit the file to keep it)", p.display());
                    }
                    None => {
                        let opts = bgws_run::tune::TuneOptions { rounds, candidates, proposer, jev_model: model, use_suite: !no_suite, suite_seeds: parse_seeds(&suite_seeds)?, ..Default::default() };
                        let r = bgws_run::tune::tune(&lab.dir, &opts, &default_figures(), &|l| eprintln!("{l}"))?;
                        println!("{}", r.report());
                        eprintln!("kept in {}", lab.dir.join("tune").join(&r.id).display());
                    }
                },
                LabCmd::Generate { only, spec } => {
                    let specs: Vec<bgws_run::scengen::Spec> = match spec {
                        Some(p) => serde_json::from_slice(&std::fs::read(&p)?)?,
                        None => bgws_run::scengen::standard(),
                    };
                    for spec in specs.into_iter().filter(|s| only.as_ref().is_none_or(|o| &s.name == o)) {
                        match bgws_run::scengen::write(&spec) {
                            Ok(p) => {
                                // Check it loads as the game would.
                                let ok = load_scenario(&p).map(|_| "ok".to_string()).unwrap_or_else(|e| format!("does not load: {e}"));
                                eprintln!("{}: {ok}", p.display());
                            }
                            Err(e) => eprintln!("{}: skipped ({e})", spec.name),
                        }
                    }
                }
                LabCmd::Wording => println!("{}", serde_json::to_string_pretty(&bgws_run::jev_wording())?),
                LabCmd::Case { moment, why } => {
                    let m = lab.moment(&moment)?;
                    let c = bgws_run::cases::case_from_moment(&m, &why)?;
                    let (bases, _) = bgws_run::cases::load(&bgws_run::cases::case_dirs(&lab.dir))?;
                    bgws_run::cases::Builder::new(bases, &default_figures())?.items(&c)?;
                    let p = bgws_run::cases::save_case(&lab.dir, &c)?;
                    eprintln!("case {} kept in {} (acceptable: {}; wrong: {})", c.id, p.display(), c.acceptable.join(", "), c.wrong.join(", "));
                }
                LabCmd::Report { points, json } => {
                    let r = lab.report()?;
                    if json {
                        println!("{}", serde_json::to_string_pretty(&r)?);
                    } else {
                        print!("{}", r.table());
                        if points {
                            println!("\n| decider | point | moments | near-best | agrees with you |\n|---|---|---|---|---|");
                            for row in r.rows.iter().filter(|r| r.point != "all") {
                                let pct = |x: Option<f64>| x.map(|v| format!("{:.0}%", v * 100.0)).unwrap_or_else(|| "–".into());
                                println!("| {} | {} | {} | {} | {} |", row.decider, row.point, row.moments, pct(row.near_best), pct(row.agrees_with_labels));
                            }
                        }
                    }
                }
            }
        }
        Cmd::Analyse { pack } => {
            let started = Instant::now();
            let last = std::sync::Mutex::new((String::new(), 0u32));
            let progress = |stage: &str, f: f64| {
                let step = (f * 10.0).floor() as u32;
                if let Ok(mut l) = last.lock() {
                    if l.0 != stage || step > l.1 {
                        *l = (stage.to_string(), step);
                        eprintln!("[{:6.1} s] {stage} {:>3.0}%", started.elapsed().as_secs_f64(), f * 100.0);
                    }
                }
            };
            let a = bgws_run::host::analyse_pack(&pack, &progress)?;
            for kind in [bgws_env::KeyKind::Observation, bgws_env::KeyKind::Chokepoint, bgws_env::KeyKind::Crossing] {
                let v: Vec<_> = a.key.iter().filter(|k| k.kind == kind).collect();
                eprintln!("{kind:?}: {}", v.len());
                for k in v.iter().take(5) {
                    eprintln!("  {:.2} at ({:.0}, {:.0}): {}", k.score, k.e, k.n, k.note);
                }
            }
            eprintln!("terrain analysis of {} in {:.0} s", pack.display(), started.elapsed().as_secs_f64());
        }
        Cmd::Pack { out, name, lat, lon, size_km, cell_m, seed, dem } => {
            let f = default_figures();
            let n = ((size_km * 1000.0 / cell_m).ceil() as usize + 1).max(16);
            let mut p = bgws_env::SynthParams::new(seed, n, n, cell_m, &f);
            p.frame_origin = bgws_core::Geodetic::new(lat, lon, 0.0);
            let mut world = bgws_env::generate(&p, &f);
            // Centre the grid on the frame origin, as generated ground is in the sim.
            let shift = cell_m * (n as f64 - 1.0) / 2.0;
            world.terrain.spec.origin_e -= shift;
            world.terrain.spec.origin_n -= shift;
            for r in [&mut world.terrain.elevation.spec, &mut world.terrain.land_cover.spec, &mut world.terrain.soil.spec, &mut world.terrain.wetness.spec, &mut world.terrain.obstacle_gap.spec] {
                r.origin_e = world.terrain.spec.origin_e;
                r.origin_n = world.terrain.spec.origin_n;
            }
            if let Some(path) = dem {
                let d = bgws_run::geotiff::Dem::read(&path)?;
                let set = bgws_run::geotiff::apply_dem(&mut world.terrain, &d);
                eprintln!("DEM {}: {} of {} nodes from the DEM", path.display(), set, n * n);
            }
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
        Cmd::Vignettes { only, n, seed, json, store, accept, backlog, html } => {
            let on_backlog = |id: &str| backlog.iter().any(|b| b == id);
            let figures = default_figures();
            let scenarios = vignette_scenarios();
            let results = experiments::run_vignettes(&figures, Seed(seed), &CheckOptions { level: 0.95, replicates: Some(n) }, only.as_deref(), &scenarios);
            let mut regressed = false;
            let mut regression = Vec::new();
            if let Some(dir) = &store {
                let st = bgws_analysis::regression::RegressionStore::open(dir).map_err(|e| anyhow!("{e}"))?;
                let ro = bgws_analysis::regression::RegressionOptions::from_figures(&figures);
                let mut verdicts = Vec::new();
                for r in &results {
                    match &accept {
                        Some(why) => {
                            st.accept(r, why, &ro).map_err(|e| anyhow!("{e}"))?;
                        }
                        None => verdicts.push(st.check_and_record(r, &ro).map_err(|e| anyhow!("{e}"))?),
                    }
                }
                regression = verdicts.clone();
                if !verdicts.is_empty() {
                    regressed = verdicts.iter().any(|v| v.status.fails() && !(on_backlog(&v.id) && v.status == bgws_analysis::regression::RegressionStatus::OutOfBand));
                    println!("{}", bgws_analysis::report::regression_markdown(&verdicts));
                }
            }
            let suite = VignetteSuite::new(results);
            if let Some(p) = &html {
                let data = serde_json::json!({ "title": "Benchmark vignettes", "generated": format!("{n} games each, seed {seed}"), "vignettes": suite.results, "regression": regression, "backlog": backlog });
                std::fs::write(p, bgws_run::workbench::html(&data))?;
                eprintln!("workbench written to {}", p.display());
            }
            println!("{}", suite.to_markdown());
            if regressed {
                bail!("a vignette regressed against its baseline (accept with --accept \"why\" if intended)");
            }
            if let Some(p) = json {
                std::fs::write(p, serde_json::to_string_pretty(&suite)?)?;
            }
            let failing: Vec<&str> = suite.results.iter().filter(|r| !matches!(r.verdict, bgws_analysis::Verdict::InBand) && !on_backlog(&r.id)).map(|r| r.id.as_str()).collect();
            let waived: Vec<&str> = suite.results.iter().filter(|r| !matches!(r.verdict, bgws_analysis::Verdict::InBand) && on_backlog(&r.id)).map(|r| r.id.as_str()).collect();
            if !waived.is_empty() {
                eprintln!("out of band, on the calibration backlog: {}", waived.join(", "));
            }
            if !failing.is_empty() {
                bail!("out of band: {}", failing.join(", "));
            }
        }
        Cmd::Events { log, sql } => {
            let t = bgws_run::eventlog::query(&log, sql.as_deref().unwrap_or(bgws_run::eventlog::SUMMARY_SQL))?;
            print!("{}", t.markdown());
        }
        Cmd::Behaviour { html } => {
            std::fs::write(&html, bgws_run::behaviour::builtin_html()?)?;
            eprintln!("Behaviour Manager written to {}", html.display());
        }
        Cmd::Compare { vignette, set, n, seed, html } => {
            let a_fig = default_figures();
            let mut b_fig = default_figures();
            for kv in &set {
                let (k, v) = kv.split_once('=').ok_or_else(|| anyhow!("--set wants key=value, got `{kv}`"))?;
                let v: f64 = v.trim().parse().map_err(|_| anyhow!("`{v}` is not a number"))?;
                let old = b_fig.try_get(k.trim()).ok_or_else(|| anyhow!("no figure `{k}`"))?;
                b_fig.set(k.trim(), v).map_err(|e| anyhow!("{e}"))?;
                eprintln!("{k}: {old} → {v}");
            }
            let scenarios = vignette_scenarios();
            let opts = CheckOptions { level: 0.95, replicates: Some(n) };
            let pick = |f: &bgws_core::Figures| experiments::run_vignettes(f, Seed(seed), &opts, Some(&vignette), &scenarios).into_iter().find(|r| r.id == vignette);
            let a = pick(&a_fig).ok_or_else(|| anyhow!("no vignette `{vignette}` with an experiment"))?;
            let b = pick(&b_fig).expect("the same vignette");
            let ks = bgws_analysis::compare::ks_two_sample(&a.sample.values, &b.sample.values);
            let mw = bgws_analysis::compare::mann_whitney(&a.sample.values, &b.sample.values);
            let label_b = set.join(", ");
            let ci = |r: &bgws_analysis::VignetteResult| r.interval.as_ref().map(|i| format!("{:.3} – {:.3}", i.lo, i.hi)).unwrap_or_else(|| "—".into());
            println!("## {}: as shipped vs {label_b}\n", a.title);
            println!("| Variant | Estimate | Interval | Band | Games | Verdict |\n|---|---|---|---|---|---|");
            for (name, r) in [("as shipped", &a), (label_b.as_str(), &b)] {
                println!("| {name} | {:.3} {} | {} | {:.3} – {:.3} | {} | {:?} |", r.estimate, r.unit, ci(r), r.band.lo, r.band.hi, r.sample.values.len(), r.verdict);
            }
            match (&ks, &mw) {
                (Some(k), Some(m)) => println!("\nKS D {:.3}, p {:.3}; Mann–Whitney p {:.3}: the two {} at 5%.", k.d, k.p_value, m.p_value, if k.p_value < 0.05 || m.p_value < 0.05 { "differ" } else { "do not detectably differ" }),
                _ => println!("\nToo few values to test."),
            }
            if let Some(p) = &html {
                let data = serde_json::json!({ "title": format!("Compare: {}", a.title), "generated": format!("{n} games each, seed {seed}"),
                    "comparison": { "a": a, "b": b, "labelA": "as shipped", "labelB": label_b, "ks": ks, "mw": mw } });
                std::fs::write(p, bgws_run::workbench::html(&data))?;
                eprintln!("workbench written to {}", p.display());
            }
        }
        Cmd::Sensitivity { vignette, replicates, prefix, seed, html } => {
            let report = bgws_run::experiments::sensitivity(&vignette, replicates, prefix.as_deref(), Seed(seed))?;
            println!("{}", bgws_analysis::report::sensitivity_markdown(&report));
            if let Some(p) = &html {
                let data = serde_json::json!({ "title": format!("Sensitivity: {vignette}"), "generated": format!("{replicates} replicates, seed {seed}"), "sensitivity": [report] });
                std::fs::write(p, bgws_run::workbench::html(&data))?;
                eprintln!("workbench written to {}", p.display());
            }
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
