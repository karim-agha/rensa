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

#[cfg(test)]
mod test;

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
  consumer::{BlockConsumer, BlockConsumers},
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

  // SUGESTION:
  // Validator::start(opt...);

  // Create the P2P networking layer.
  // Networking runs on its own separate thread,
  // and emits events by calling .poll()
  let mut network = Network::new(
    &genesis,
    crate::network::create_tcp_transport(&opts.keypair).await?,
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
    opts.blocks_history(),
    opts.rpc_endpoints().is_some(), // store tx details only if RPC is enabled
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
  let mut producer = BlockProducer::new(&genesis, opts.keypair.clone());
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
  let mut consumers: Vec<Arc<dyn BlockConsumer<_>>> = vec![
    // persists blocks that have been confirmed or finalized
    Arc::new(blocks_store.clone()),
    // remove all observed votes and txs from the mempool that
    // were included by other validators.
    Arc::new(producer.clone()),
  ];

  // dbsync is an optional opt-in feature
  if let Some(dbsync) = opts.dbsync().await? {
    consumers.push(Arc::new(dbsync));
  }

  let consumers = BlockConsumers::new(consumers);

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
        chain.with_head(|state, block| {
          debug!("[slot {}]: {} is considered head of chain @ height {}",
            slot, block.hash().unwrap().to_b58(), block.height());
          if validator.pubkey == me {
            producer.produce(state, block, &vm);
          }
        });
      }

      // this node should respond with a block reply for
      // a requested hash. This is the rate limiter component.
      Some(block_hash) = block_reply_responder.next() => {
        if let Some(block) = chain
          .get(block_hash)
          .cloned()
          .or_else(|| blocks_store.get_by_hash(&block_hash).map(|(b, _)| b))
        {
          info!("Replaying block {}", &*block);
          network.gossip_block((*block.underlying).clone())?
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
          NetworkEvent::TransactionReceived(tx) => {
            producer.record_transaction(tx);
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
          ChainEvent::BlockDiscarded(block) => {
            info!("discarded block {block}");
            producer.reuse_discarded(block);
          }
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

            // run all consumers for this block on a separate thread
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

      // RPC API
      //  The only RPC events that modify the state, and thus
      //  relevant for the validator loop are new transactions,
      //  all other RPC calls are read-only operations that retreive
      //  information about and from the chain or storage.
      //
      // When a transaction arrives through RPC, immediately propagate
      // it through gossip to validators to be picked up by all validators
      // mempools
      Some(tx) = apisvc.next() => {
        debug!("Transaction received throught RPC: {tx:?}");
        network.gossip_transaction(tx)?;
      }
    }
  }
}
