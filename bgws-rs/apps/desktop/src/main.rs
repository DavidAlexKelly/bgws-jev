//! `bgws-desktop`: the desktop shell (RUST_ENGINE.md §16). The host and the
//! game run in this process, on their own thread, as `bgws-server` does; the
//! window shows the same viewer, with the editor and the Behaviour Manager a
//! link away. The page talks to the host over a WebSocket on a private
//! localhost port: commands one way, JSON frames with binary position
//! updates the other.
//!
//! ```text
//! bgws-desktop [scenario] [--deciders rules|jev|claude|local] [--person blue] [--seed N] [--flat]
//! ```

use clap::Parser;
use std::sync::mpsc;
use std::time::Duration;
use tauri::webview::PageLoadEvent;
use tauri::{WebviewUrl, WebviewWindowBuilder};

#[derive(Parser, Debug)]
#[command(name = "bgws-desktop")]
struct Cli {
    /// A shipped scenario's name or a scenario file.
    #[arg(default_value = "skirmish-advance-to-contact-v1")]
    scenario: String,
    #[arg(long, default_value = "scenario")]
    deciders: String,
    #[arg(long)]
    person: Option<String>,
    #[arg(long)]
    seed: Option<u64>,
    #[arg(long)]
    flat: bool,
    /// Close this many seconds after the page has loaded (a check that the shell runs).
    #[arg(long)]
    smoke_s: Option<f64>,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    // The server's own arguments, on a free port.
    let mut argv = vec!["bgws-server".to_string(), cli.scenario.clone(), "--port".into(), "0".into(), "--deciders".into(), cli.deciders.clone()];
    if let Some(p) = &cli.person {
        argv.extend(["--person".into(), p.clone()]);
    }
    if let Some(s) = cli.seed {
        argv.extend(["--seed".into(), s.to_string()]);
    }
    if cli.flat {
        argv.push("--flat".into());
    }
    let args = bgws_server::Args::try_parse_from(argv)?;

    // The host, in-process, on its own runtime.
    let (tx, rx) = mpsc::channel();
    let failed = tx.clone();
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
            Ok(rt) => rt,
            Err(e) => return drop(failed.send(Err(e.to_string()))),
        };
        if let Err(e) = rt.block_on(bgws_server::serve(args, move |addr| drop(tx.send(Ok(addr))))) {
            let _ = failed.send(Err(e.to_string()));
        }
    });
    let addr = rx.recv_timeout(Duration::from_secs(120)).map_err(|_| anyhow::anyhow!("the host did not start"))?.map_err(|e| anyhow::anyhow!("the host failed: {e}"))?;
    let url: tauri::Url = format!("http://{addr}/").parse()?;
    eprintln!("bgws-desktop: host at {url}");

    let smoke = cli.smoke_s;
    tauri::Builder::default()
        .setup(move |app| {
            let handle = app.handle().clone();
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
                .title("BGWS")
                .inner_size(1400.0, 900.0)
                .on_page_load(move |window, payload| {
                    if payload.event() != PageLoadEvent::Finished {
                        return;
                    }
                    eprintln!("bgws-desktop: loaded {}", payload.url());
                    if let Some(s) = smoke {
                        let handle = handle.clone();
                        let _ = window.eval("document.title = 'BGWS · ' + (typeof frame === 'undefined' ? 'no frame' : 'viewer')");
                        std::thread::spawn(move || {
                            std::thread::sleep(Duration::from_secs_f64(s));
                            handle.exit(0);
                        });
                    }
                })
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())?;
    Ok(())
}
