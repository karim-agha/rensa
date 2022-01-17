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
use network::Network;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{info, Level};

fn print_essentials(opts: &CliOpts) {
  info!("Starting Rensa validator node");
  info!("Version: {}", env!("CARGO_PKG_VERSION"));
  info!("Listen addresses: {:?}", opts.listen_multiaddrs());
  info!("Chain identity: {}", opts.keypair);
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

  let mut network = Network::new(
    "rensta-t1",
    opts.keypair.clone(),
    opts.listen_multiaddrs().into_iter(),
  )
  .await?;

  // connect to bootstrap nodes if specified
  opts
    .peers()
    .into_iter()
    .for_each(|p| network.connect(p).unwrap());

  let (ticks_tx, mut ticks_rx) = mpsc::unbounded_channel::<u128>();

  tokio::spawn(async move {
    let mut counter = 0;
    loop {
      tokio::time::sleep(Duration::from_secs(3)).await;
      ticks_tx.send(counter).unwrap();
      counter += 1;
    }
  });

  loop {
    tokio::select! {
      Some(event) = network.next() => {
        info!("network event: {:?}", event);
      },
      Some(tick) = ticks_rx.recv() => {
        network.gossip(tick.to_le_bytes().to_vec())?;
      }
    }
  }
}
