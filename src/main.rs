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
  network::{responder::SwarmResponder, Network},
  producer::BlockProducer,
  rpc::ApiService,
  std::sync::Arc,
  storage::{BlockStore, PersistentState},
  tracing::{debug, info, Level},
  tracing_subscriber::{
    filter::filter_fn,
    prelude::__tracing_subscriber_SubscriberExt,
    util::SubscriberInitExt,
    Layer,
  },
  vm::Finalized,
};

#[global_allocator]
static ALLOC: jemallocator::Jemalloc = jemallocator::Jemalloc;

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

  let loglevel = match opts.verbose {
    1 => Level::DEBUG,
    2 => Level::TRACE,
    _ => Level::INFO,
  };
  tracing_subscriber::registry()
    .with(tracing_subscriber::fmt::layer().with_filter(filter_fn(
      move |metadata| {
        !metadata.target().starts_with("netlink")
          && metadata.level() <= &loglevel
      },
    )))
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
  let blocks_store = BlockStore::new(
    opts.data_dir()?, // storage dir root
    opts.replay_blocks_len(),
  )?;

  // get the latest finalized block that this validator is aware of
  // so far. It is is the first run of a validator, then it is going
  // to be the genesis block.
  let latest_block: Arc<dyn Block<_>> =
    match blocks_store.latest(Commitment::Finalized) {
      Some(b) => Arc::new(b),
      None => Arc::new(genesis.clone()),
    };

  // The finalized state and block that graduated from consensus
  // and is guaranteed to never be overriten on any validator beyond
  // this point by any forkchoice rules.
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
    // exports data changes to external DBs
    Box::new(DatabaseSync::new()),
    // persists blocks that have been confirmed or finalized
    Box::new(blocks_store.clone()),
  ]);

  // responsible for deciding if the current node should
  // respond to  block reply requests.
  let mut block_reply_responder = SwarmResponder::new(
    genesis.slot_interval, // minimum delay
    genesis.validators.len(),
  ); // max delay = log2(network) * min

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

      // this node should respond with a block reply for
      // a requested hash. This is the rate limiter component.
      Some(block_hash) = block_reply_responder.next() => {
        if let Some(block) = chain
          .get(&block_hash)
          .cloned()
          .or_else(|| blocks_store.get(&block_hash).map(|(b, _)| b))
        {
          info!("Replaying block {block}");
          network.gossip_block(block)?
        }
      }

      // Networking worker, receives
      // data from p2p gossip between validators
      Some(event) = network.poll() => {
        match event {
          NetworkEvent::BlockReceived(block) => {
            if let Ok(hash) = block.hash() {
              block_reply_responder.cancel(&hash);
            }
            chain.include(block);
          },
          NetworkEvent::VoteReceived(vote) => {
            producer.record_vote(vote);
          },
          NetworkEvent::MissingBlock(block_hash) => {
           block_reply_responder.request(block_hash);
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
          ChainEvent::BlockIncluded(block) => {
            info!(
              "included block {} [epoch {}] [state hash: {}]",
              *block, block.height() / genesis.epoch_blocks,
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
              block.height() / genesis.epoch_blocks,
              block.state().hash().to_bytes().to_b58()
            );
            consumers.consume(block, Commitment::Confirmed)?;
          }
          ChainEvent::BlockFinalized { block, votes } => {
            info!(
              "finalized block {} with {:.02}% votes [epoch {}] [state hash: {}]",
              *block,
              (votes as f64 * 100f64) / chain.total_stake() as f64,
              block.height() / genesis.epoch_blocks,
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
