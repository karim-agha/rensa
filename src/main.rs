mod cli;
pub mod consensus;
pub mod network;
pub mod primitives;
pub mod producer;
pub mod rpc;
pub mod storage;
pub mod vm;

use std::rc::Rc;

use crate::{
  consensus::{block::Block, chain::ChainEvent},
  network::NetworkEvent,
  primitives::ToBase58String,
};
use clap::StructOpt;
use cli::CliOpts;
use consensus::{
  chain::Chain,
  schedule::{ValidatorSchedule, ValidatorScheduleStream},
  vote::Vote,
};
use futures::StreamExt;
use network::Network;
use producer::BlockProducer;
use tracing::{info, Level};
use vm::{Finalized, FinalizedState};

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

  // Create the P2P networking layer.
  // Networking runs on its own separate thread,
  // and emits events by calling .poll()
  let mut network = Network::new(
    &genesis,
    opts.keypair.clone(),
    opts.listen_multiaddrs().into_iter(),
  )
  .await
  .unwrap();

  // connect to bootstrap nodes if specified
  for peer in opts.peers() {
    network.connect(peer)?;
  }

  let me = opts.keypair.public();
  let seed = genesis.hash()?.digest().try_into()?;

  // the blockchain state.
  // Persistance is not implemented yet, so using
  // the gensis block as the last finalized block
  let finalized = Finalized {
    underlying: Rc::new(genesis.clone()),
    state: FinalizedState,
  };

  // componsents of the consensus
  let mut chain = Chain::new(&genesis, finalized);
  let mut producer = BlockProducer::new(&genesis, opts.keypair.clone());
  let mut schedule = ValidatorScheduleStream::new(
    ValidatorSchedule::new(seed, &genesis.validators)?,
    genesis.genesis_time,
    genesis.slot_interval,
  );

  // validator runloop
  loop {
    tokio::select! {
      Some((slot, validator)) = schedule.next() => {
        let head = chain.head();
        info!("[slot {}]: {} is considered head of chain @ height {}",
          slot, head.hash()?.to_b58(), head.height());
        if validator.pubkey == me {
          producer.produce(slot, head);
        }
      }
      Some(event) = network.poll() => {
        match event {
          NetworkEvent::BlockReceived(block) => {
            chain.include(block);
          },
          NetworkEvent::VoteReceived(vote) => {
            producer.record_vote(vote);
          },
        }
      },
      Some(block) = producer.next() => {
        chain.include(block.clone());
        network.gossip_block(block)?;
      }
      Some(event) = chain.next() => {
        match event {
          ChainEvent::Vote { target, justification } => {
            network.gossip_vote(Vote::new(
              &opts.keypair,
              target,
              justification))?;
          },
          ChainEvent::BlockIncluded(block) => {
            info!("Block {block} included successfully.");

            // don't duplicate votes if they were
            // already included be an accepted block.
            producer.exclude_votes(&block);
          }
        }
      }
    }
  }
}
