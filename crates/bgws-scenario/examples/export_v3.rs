//! Convert TS v3 scenario files to v4 (the shipped `data/scenarios/` were made with this):
//! `cargo run -p bgws-scenario --example export_v3 -- <v3 dir> <out dir>`.
//! Each file is converted with the default options (TS origin, authored metres
//! placed through an ENU frame) and validated against the built-in catalogue.

use bgws_catalogue::Catalogue;
use bgws_scenario::{convert_v3, parse_v3, validate, ConvertOptions};
use std::path::Path;

fn slug(name: &str) -> String {
    let s: String = name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-")
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: export_v3 <v3 dir> <out dir>");
        std::process::exit(2);
    }
    let cat = match Catalogue::builtin() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("catalogue: {e}");
            std::process::exit(1);
        }
    };
    let mut entries: Vec<_> = match std::fs::read_dir(&args[1]) {
        Ok(rd) => rd.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect(),
        Err(e) => {
            eprintln!("{}: {e}", args[1]);
            std::process::exit(1);
        }
    };
    entries.sort();
    let mut failed = false;
    for path in entries {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let raw: serde_json::Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("{}: {e}", path.display());
                failed = true;
                continue;
            }
        };
        let v3 = match parse_v3(&raw) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("{}: {}", path.display(), e.join("; "));
                failed = true;
                continue;
            }
        };
        let v4 = match convert_v3(&v3, Some(&cat), &ConvertOptions::default()) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("{}: {e}", path.display());
                failed = true;
                continue;
            }
        };
        let problems = validate(&v4, &cat);
        for p in &problems {
            eprintln!("{}: {p}", path.display());
        }
        failed |= !problems.is_empty();
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("scenario");
        let name = if stem.starts_with("skirmish-") { stem.to_owned() } else { slug(&v4.name) };
        let out = Path::new(&args[2]).join(format!("{name}.json"));
        if let Err(e) = v4.save(&out) {
            eprintln!("{e}");
            failed = true;
        } else {
            println!("{} -> {} ({} forces)", path.display(), out.display(), v4.forces.len());
        }
    }
    if failed {
        std::process::exit(1);
    }
}
