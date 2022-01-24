mod cli;
pub mod consensus;
pub mod network;
pub mod primitives;
pub mod rpc;
pub mod state;
pub mod storage;
use crate::{
  consensus::block::Block, network::NetworkEvent, primitives::ToBase58String,
};
use clap::StructOpt;
use cli::CliOpts;
use consensus::{
  chain::Chain,
  producer::BlockProducer,
  schedule::{ValidatorSchedule, ValidatorScheduleStream},
  vote::VoteProducer,
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
  info!("Genesis hash: {}", genesis.hash()?.to_b58());

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

  // the blockchain, no persistance yet
  let mut chain = Chain::new(&genesis);

  // componsents of the consensus
  let mut voter = VoteProducer::new(&chain);
  let mut producer = BlockProducer::new(opts.keypair);
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
          NetworkEvent::BlockReceived(block) => chain.include(block),
          NetworkEvent::VoteReceived(vote) => chain.vote(vote),
        }
      },
      Some(block) = producer.next() => {
        chain.include(block.clone());
        network.gossip_block(&block)?;
      }
      Some(vote) = voter.next() => {
        chain.vote(vote.clone());
        network.gossip_vote(&vote)?;
      }
      Some((slot, validator)) = schedule.next() => {
        if validator.pubkey == me {
          producer.produce(slot, match chain.head() {
            Some(b) => b,
            None => chain.genesis(),
          });
        }
      }
    }
  }
}
