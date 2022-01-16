mod cli;
pub mod consensus;
pub mod execution;
pub mod keys;
pub mod network;
pub mod rpc;
pub mod storage;
pub mod transaction;

use clap::StructOpt;
use cli::CliOpts;
use futures::StreamExt;
use libp2p::{
  identify::{Identify, IdentifyConfig},
  Swarm,
};
use tracing::{info, Level};

fn print_essentials(opts: &CliOpts) {
  info!("Starting Rensa validator node");
  info!("Version: {}", env!("CARGO_PKG_VERSION"));
  info!("Listen address: {}", opts.listen_multiaddr());
  info!("Chain identity: {}", opts.secret);
  info!(
    "P2P identity: {}",
    opts.p2p_identity().public().to_peer_id()
  );
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  let opts = CliOpts::parse();

  tracing_subscriber::fmt()
    .with_max_level(match opts.verbose {
      1 => Level::DEBUG,
      2 => Level::TRACE,
      _ => Level::INFO,
    })
    .init();

  print_essentials(&opts);

  let mut swarm = Swarm::new(
    network::create_transport(&opts.secret).await?,
    Identify::new(IdentifyConfig::new(
      "/rensa/".to_owned(),
      opts.p2p_identity().public(),
    )),
    opts.p2p_identity().public().to_peer_id(),
  );

  while let Some(event) = swarm.next().await {
    info!("swarm event: {:?}", event);
  }

  Ok(())
}
