mod cli;
pub mod consensus;
pub mod execution;
pub mod keys;
pub mod network;
pub mod rpc;
pub mod storage;
pub mod transaction;

use crate::{consensus::block::Block, network::NetworkEvent};
use clap::StructOpt;
use cli::CliOpts;
use consensus::{
  consumer::BlockConsumer,
  producer::BlockProducer,
  schedule::{ValidatorSchedule, ValidatorScheduleStream},
  vote::{VoteConsumer, VoteProducer}, chain::Chain,
};
use futures::StreamExt;
use network::Network;
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

  let genesis = opts.genesis()?;

  info!("Genesis: {:#?}", genesis);
  info!(
    "Genesis hash: {}",
    bs58::encode(&genesis.hash()?.to_bytes()).into_string()
  );

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

  // Create the P2P networking layer
  let mut network = Network::new(
    &genesis,
    opts.keypair.clone(),
    opts.listen_multiaddrs().into_iter(),
  )
  .await?;

  // connect to bootstrap nodes if specified
  opts
    .peers()
    .into_iter()
    .for_each(|p| network.connect(p).unwrap());

  let me = opts.keypair.public();
  let seed = genesis.hash()?.digest().try_into()?;

  
  // the blockchain
  let chain = Chain::new(&genesis);
  
  // componsents of the consensus
  let mut voter = VoteProducer::new(&chain);
  let mut ballot = VoteConsumer::new(&chain);
  let mut consumer = BlockConsumer::new(&chain);
  let mut producer = BlockProducer::new(&chain);
  let mut schedule = ValidatorScheduleStream::new(
    ValidatorSchedule::new(seed, &genesis.validators)?,
    genesis.genesis_time,
    genesis.slot_interval,
  );

  // validator runloop
  loop {
    tokio::select! {
      Some(event) = network.next() => {
        match event {
          NetworkEvent::BlockReceived(block) => consumer.consume(block),
          NetworkEvent::VoteReceived(vote) => ballot.consume(vote),
        }
      },
      Some(block) = producer.next() => {
        info!("block produced at height {}", block.height());
        network.gossip_block(&block)?;
      }
      Some(vote) = voter.next() => {
        info!("vote received {vote:?}");
        network.gossip_vote(&vote)?;
      }
      Some((slot, validator)) = schedule.next() => {
        if validator.pubkey == me {
          info!("It's my turn on slot {slot}: {validator:?}");
          producer.produce(slot);
        } else {
          info!("I think that slot {slot} is for: {validator:?}");
        }
      }
    }
  }
}
