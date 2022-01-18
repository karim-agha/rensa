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
use consensus::validator::{ValidatorSchedule, ValidatorScheduleStream};
use futures::StreamExt;
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
    &genesis.validators,
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
    let me = opts.keypair.public();
    let validators = genesis.validators.clone();

    let mut schedule = ValidatorSchedule::new(seed, &validators).unwrap();
    let mut schedule_stream = ValidatorScheduleStream::new(
      &mut schedule,
      genesis.genesis_time,
      genesis.slot_interval,
    );

    while let Some((slot, validator)) = schedule_stream.next().await {
      if validator.pubkey == me {
        ticks_tx.send(validator.pubkey.clone()).unwrap();
        info!("It's my turn on slot {slot}: {validator:?}");
      } else {
        info!("I think that slot {slot} is for: {validator:?}");
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
