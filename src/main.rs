mod cli;
pub mod consensus;
pub mod execution;
pub mod keys;
pub mod network;
pub mod rpc;
pub mod storage;
pub mod transaction;

use chrono::Utc;
use clap::StructOpt;
use cli::CliOpts;
use consensus::validator::ValidatorSchedule;
use keys::Pubkey;
use network::Network;
use tokio::sync::mpsc;
use tracing::{info, Level};

fn print_essentials(opts: &CliOpts) -> anyhow::Result<()> {
  info!("Starting Rensa validator node");
  info!("Version: {}", env!("CARGO_PKG_VERSION"));
  info!("Listen addresses: {:?}", opts.listen_multiaddrs());
  info!("Chain identity: {}", opts.keypair);
  info!(
    "P2P identity: {}",
    opts.p2p_identity().public().to_peer_id()
  );
  info!("Genesis: {:#?}", opts.genesis()?);

  Ok(())
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

  // print basic information about the
  // validator software and the blockchain
  print_essentials(&opts)?;

  // read the genesis configuration
  let genesis = opts.genesis()?;

  let mut network = Network::new(
    genesis.chain_id,
    opts.keypair.clone(),
    opts.listen_multiaddrs().into_iter(),
  )
  .await?;

  // connect to bootstrap nodes if specified
  opts
    .peers()
    .into_iter()
    .for_each(|p| network.connect(p).unwrap());

  let (ticks_tx, mut ticks_rx) = mpsc::unbounded_channel::<Pubkey>();

  tokio::spawn(async move {
    let seed = [5u8; 32];
    let validators = genesis.validators.clone();

    let slots_since_genesis = (Utc::now().timestamp_millis()
      - genesis.genesis_time.timestamp_millis())
      as u128
      / genesis.slot_interval.as_millis();
    info!("current slot number: {slots_since_genesis}");
    let schedule = ValidatorSchedule::new(seed, &validators).unwrap();
    let mut current = schedule.skip(slots_since_genesis as usize);
    loop {
      tokio::time::sleep(genesis.slot_interval).await;
      if let Some(validator) = current.next() {
        ticks_tx.send(validator.pubkey.clone()).unwrap();
        info!("validator turn: {validator:?}");
      }
    }
  });

  loop {
    tokio::select! {
      Some(event) = network.next() => {
        info!("network event: {:?}", event);
      },
      Some(tick) = ticks_rx.recv() => {
        network.gossip(tick.to_vec())?;
      }
    }
  }
}
