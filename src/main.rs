use tracing_subscriber::EnvFilter;

mod cli;
mod consensus;
mod consumer;
mod dbsync;
mod network;
mod primitives;
mod producer;
mod rpc;
mod storage;
mod vm;

use {
  crate::{
    consumer::Commitment,
    network::NetworkEvent,
    primitives::{OptionalStreamExt, ToBase58String},
    vm::State,
  },
  clap::StructOpt,
  cli::CliOpts,
  consensus::{
    Block,
    Chain,
    ChainEvent,
    ValidatorSchedule,
    ValidatorScheduleStream,
    Vote,
  },
  consumer::BlockConsumers,
  dbsync::DatabaseSync,
  futures::StreamExt,
  network::Network,
  producer::BlockProducer,
  rpc::ApiService,
  storage::{BlockStore, PersistentState},
  tracing::{debug, info, Level},
  vm::Finalized,
};

fn print_essentials(opts: &CliOpts) -> anyhow::Result<()> {
  info!("Starting Rensa validator node");
  info!("Version: {}", env!("CARGO_PKG_VERSION"));
  info!("Listen addresses: {:?}", opts.listen_multiaddrs());
  info!("Chain identity: {}", opts.keypair);
  info!("Data directory: {}", opts.data_dir()?.display());
  info!(
    "P2P identity: {}",
    opts.p2p_identity().public().to_peer_id()
  );

  let genesis = opts.genesis()?;

  info!("Genesis: {:#?}", genesis);
  info!("Genesis hash: {}", genesis.hash()?.to_b58());

  Ok(())
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
  let opts = CliOpts::parse();

  tracing_subscriber::fmt()
    .with_env_filter(EnvFilter::from_default_env())
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

  // The blockchain state storage. This survives crashes, and
  // anything that gets here has went thorugh the complete consensus
  // process.
  let storage = PersistentState::new(&genesis, opts.data_dir()?)?;
  let blocks_store = BlockStore::new(opts.data_dir()?)?;

  // get the latest finalized block that this validator is aware of
  // so far. It is is the first run of a validator, then it is going
  // to be the genesis block.
  let latest_block: Box<dyn Block<_>> =
    match blocks_store.latest(Commitment::Finalized) {
      Some(b) => Box::new(b),
      None => Box::new(genesis.clone()),
    };

  let finalized = Finalized::new(latest_block, &storage);

  // the transaction processing runtime
  let vm = vm::Machine::new(&genesis)?;

  // components of the consensus
  let mut chain = Chain::new(&genesis, &vm, finalized);
  let mut producer = BlockProducer::new(&genesis, &vm, opts.keypair.clone());
  let mut schedule = ValidatorScheduleStream::new(
    ValidatorSchedule::new(seed, &genesis)?,
    genesis.genesis_time,
    genesis.slot_interval,
  );

  // external client JSON API
  let mut apisvc = opts.rpc_endpoints().map(|addrs| {
    Box::new(ApiService::new(
      addrs,
      storage.clone(),
      blocks_store.clone(),
      genesis.clone(),
    ))
  });

  // those are components that ingest newly included,
  // confirmed and finalized blocks
  let consumers = BlockConsumers::new(vec![
    Box::new(DatabaseSync::new()), // exports data changes to external DBs
    Box::new(blocks_store.clone()), // persists blocks that have been finalized
  ]);

  // validator runloop
  loop {
    tokio::select! {

      // core services:

      // Validator Schedule worker, responsible for
      // signalling that a new slot started and who's
      // turn it is for the current slot.
      Some((slot, validator)) = schedule.next() => {
        let (state, block) = chain.head();
        debug!("[slot {}]: {} is considered head of chain @ height {}",
          slot, block.hash()?.to_b58(), block.height());
        if validator.pubkey == me {
          producer.produce(slot, state, block);
        }
      }

      // Networking worker, receives
      // data from p2p gossip between validators
      Some(event) = network.poll() => {
        match event {
          NetworkEvent::BlockReceived(block) => {
            chain.include(block);
          },
          NetworkEvent::VoteReceived(vote) => {
            producer.record_vote(vote);
          },
          NetworkEvent::MissingBlock(block_hash) => {
            chain.try_replay_block(block_hash);
          }
        }
      },

      // Block producer, builds new block when
      // it is current validators turn to produce
      // a new block
      Some(block) = producer.next() => {
        chain.include(block.clone());
        network.gossip_block(block)?;
      }

      // Events generated by the consensus algorithm
      Some(event) = chain.next() => {
        match event {
          ChainEvent::Vote { target, justification } => {
            network.gossip_vote(Vote::new(
              &opts.keypair,
              target,
              justification))?;
          },
          ChainEvent::BlockMissing(hash) => {
            info!(
              "Block {} is missing, requesting replay.",
              hash.to_bytes().to_b58()
            );
            network.gossip_missing(hash)?
          }
          ChainEvent::BlockReplayed(block) => {
            info!("Replaying block {block}");
            network.gossip_block(block)?
          }
          ChainEvent::BlockIncluded(block) => {
            info!(
              "included block {} [epoch {}] [state hash: {}]",
              *block, block.slot() / genesis.epoch_slots,
              block.state().hash().to_bytes().to_b58()
            );
            // don't duplicate votes if they were
            // already included be an accepted block.
            producer.exclude_votes(&block);
            consumers.consume(block, Commitment::Included)?;
          }
          ChainEvent::BlockConfirmed { block, votes } => {
            info!(
              "confirmed block {} with {:.02}% votes [epoch {}] [state hash: {}]",
              *block,
              (votes as f64 * 100f64) / chain.total_stake() as f64,
              block.slot() / genesis.epoch_slots,
              block.state().hash().to_bytes().to_b58()
            );
            consumers.consume(block, Commitment::Confirmed)?;
          }
          ChainEvent::BlockFinalized { block, votes } => {
            info!(
              "finalized block {} with {:.02}% votes [epoch {}] [state hash: {}]",
              *block,
              (votes as f64 * 100f64) / chain.total_stake() as f64,
              block.slot() / genesis.epoch_slots,
              block.state().hash().to_bytes().to_b58()
            );
            consumers.consume(block, Commitment::Finalized)?;
          }
        }
      }

      // optional services:

      // RPC API events
      Some(event) = apisvc.next() => {
        info!("RPC Event: {event:?}");
      }
    }
  }
}
