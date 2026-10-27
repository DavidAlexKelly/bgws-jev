//! `bgws-server`: the game and its pages on a local port (see the library).

use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    bgws_server::serve(bgws_server::Args::parse(), |addr| eprintln!("bgws-server: open http://{addr}/")).await
}
