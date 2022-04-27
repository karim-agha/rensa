use {
  crate::{
    consensus::{
      block::Block,
      validator::Validator,
      Chain,
      ChainEvent,
      Genesis,
      ValidatorSchedule,
      ValidatorScheduleStream,
      Vote,
    },
    consumer::{BlockConsumer, BlockConsumers, Commitment},
    network::{
      responder::SwarmResponder,
      Network,
      NetworkCommand,
      NetworkEvent,
    },
    primitives::Keypair,
    producer::BlockProducer,
    test::in_mem_state::InMemState,
    vm::{self, Executed, Finalized, Transaction},
  },
  async_trait::async_trait,
  futures::StreamExt,
  libp2p::{multiaddr::Protocol, Multiaddr},
  multihash::Multihash,
  std::{collections::HashMap, sync::Arc},
  thiserror::Error,
  tokio::sync::{mpsc::error::SendError, RwLock},
};

#[derive(Debug, Error)]
pub enum MemValidatorError {
  #[error(transparent)]
  IOError(#[from] std::io::Error),
  #[error(transparent)]
  CommandSendError(#[from] SendError<NetworkCommand<Vec<Transaction>>>),
  #[error(transparent)]
  ExecutedSendError(
    #[from] SendError<(Executed<Vec<Transaction>>, Commitment)>,
  ),
}

pub type Result<T> = std::result::Result<T, MemValidatorError>;

pub type BlockStoreDb = HashMap<Multihash, Executed<Vec<Transaction>>>;

#[derive(Default, Clone)]
struct BlockStore {
  db: Arc<RwLock<BlockStoreDb>>,
}

impl BlockStore {
  async fn get_by_hash(
    &self,
    hash: &Multihash,
  ) -> Option<Executed<Vec<Transaction>>> {
    self.db.read().await.get(hash).cloned()
  }
}

#[async_trait]
impl BlockConsumer<Vec<Transaction>> for BlockStore {
  async fn consume(
    &self,
    block: Executed<Vec<Transaction>>,
    _commitment: Commitment,
  ) {
    // NOTE(bmaas): looked at the block.hash() method, this returns a
    // result the question becomes how to handle this on consumption
    // here.
    self.db.write().await.insert(block.hash().unwrap(), block);
  }
}

// NOTE(bmaas): tried to implement MemValidator over a generic, but as our
// blockproducer works on Vec<Transaction> this is not possible.
pub struct MemValidator {
  genesis: Genesis<Vec<Transaction>>,
  keypair: Keypair,
  listenaddr: u64,
}

impl MemValidator {
  fn new(
    genesis: Genesis<Vec<Transaction>>,
    keypair: Keypair,
    listenaddr: usize,
  ) -> Self {
    Self {
      genesis,
      keypair,
      listenaddr: listenaddr as u64,
    }
  }

  pub fn listenaddr(&self) -> Multiaddr {
    let mut m = Multiaddr::empty();
    m.push(Protocol::Memory(self.listenaddr));
    m
  }

  pub async fn start(self, peers: Vec<Multiaddr>) -> Result<()> {
    // Create the P2P networking layer.
    // Networking runs on its own separate thread,
    // and emits events by calling .poll()

    // lets build a listenaddr for this peer
    // based on something
    let listenaddr = vec![self.listenaddr()];

    // for testing we can use the memory transport
    let mut network = Network::new(
      &self.genesis,
      crate::network::create_memory_transport(&self.keypair),
      self.keypair.clone(),
      listenaddr.into_iter(),
    )
    .await
    .unwrap();

    // connect to bootstrap nodes if specified
    for peer in peers {
      network.connect(peer)?;
    }

    let me = self.keypair.public();
    let seed: [u8; 32] = self
      .genesis
      .hash()?
      .digest()
      .try_into()
      .expect("cannot convert genesis hash into seed");

    // The validator state storage, we use in memory as we
    // are not interested in storing test validator state on disk
    let storage = InMemState::default();
    let blocks_store = BlockStore::default();

    // since we have no replay, our latest block is always based
    // on our genesis
    let latest_block: Arc<dyn Block<_>> = Arc::new(self.genesis.clone());

    // The finalized state and block that graduated from consensus
    // and is guaranteed to never be overriten on any validator beyond
    // this point by any forkchoice rules.
    let finalized = Finalized::new(latest_block, &storage);

    // the transaction processing runtime
    let vm = vm::Machine::new(&self.genesis)
      .expect("could not initialize the virtual machine");

    // components of the consensus
    let mut chain = Chain::new(&self.genesis, &vm, finalized);
    let mut producer = BlockProducer::new(&self.genesis, self.keypair.clone());
    let mut schedule = ValidatorScheduleStream::new(
      ValidatorSchedule::new(seed, &self.genesis)
        .expect("could not initialize the validator schedule"),
      self.genesis.genesis_time,
      self.genesis.slot_interval,
    );

    //

    // those are components that ingest newly included,
    // confirmed and finalized blocks
    let consumers = BlockConsumers::new(vec![
      // remove all observed votes and txs from the mempool that
      // were included by other validators.
      Arc::new(producer.clone()),
      // persists blocks that have been confirmed or finalized
      Arc::new(blocks_store.clone()),
    ]);

    // responsible for deciding if the current node should
    // respond to  block reply requests.
    let mut block_reply_responder = SwarmResponder::new(
      self.genesis.slot_interval, // minimum delay
      self.genesis.validators.len(),
    ); // max delay = log2(network) * min

    // main validator loop
    loop {
      tokio::select! {

        // core services:

        // Validator Schedule worker, responsible for
        // signalling that a new slot started and who's
        // turn it is for the current slot.
        Some((_slot, validator)) = schedule.next() => {
          chain.with_head(|state, block| {
            if validator.pubkey == me {
              producer.produce(state, block, &vm);
            }
          });
        }

        // this node should respond with a block reply for
        // a requested hash. This is the rate limiter component.
        Some(block_hash) = block_reply_responder.next() => {
           if let Some(block) = chain
            .get(&block_hash)
            .cloned()
          {
            network.gossip_block((*block.underlying).clone())?
          } else if let Some(block) = blocks_store.get_by_hash(&block_hash).await {
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
                &self.keypair,
                target,
                justification))?;
            },
            ChainEvent::BlockDiscarded(block) => {
              producer.reuse_discarded(block);
            }
            ChainEvent::BlockMissing(hash) => {
              network.gossip_missing(hash)?
            }
            ChainEvent::BlockIncluded(block) => {
              consumers.consume(block, Commitment::Included)?;
            }
            ChainEvent::BlockConfirmed { block, .. } => {
              consumers.consume(block, Commitment::Confirmed)?;
            }
            ChainEvent::BlockFinalized { block, .. } => {
              consumers.consume(block, Commitment::Finalized)?;
            }
          }
        }

      }
    }
  }
}

struct TValidator {
  keypair: Keypair,
  stake: u64,
}

impl TValidator {
  fn unique() -> Self {
    Self {
      keypair: Keypair::unique(),
      stake: 2000,
    }
  }
}

impl From<&TValidator> for Validator {
  fn from(tval: &TValidator) -> Self {
    Self {
      pubkey: tval.keypair.public(),
      stake: tval.stake,
    }
  }
}

#[cfg(test)]
mod tests {
  use {
    super::*,
    crate::test::utils::{genesis_validators, keypair_default},
  };

  #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
  async fn mem_validator_test() {
    // build some unique tvalidators, and use this to build a genesis with
    // standard validators
    let validators: Vec<_> = std::iter::repeat_with(|| TValidator::unique())
      .take(10)
      .collect();
    let genesis =
      genesis_validators(validators.iter().map(Validator::from).collect());

    // build a set of mem validators using the genesis
    let mem_validators: Vec<_> = validators
      .iter()
      .enumerate()
      .map(|(idx, tval)| {
        MemValidator::new(genesis.clone(), tval.keypair.clone(), idx)
      })
      .collect();

    // first 3 are bootstrap
    let bootstrap_nodes: Vec<_> = mem_validators
      .iter()
      .take(3)
      .map(|n| n.listenaddr())
      .collect();

    // now lets generate the peers list and start connecting
    for v in mem_validators {
      tokio::spawn(v.start(bootstrap_nodes.clone()));
      // v.start(bootstrap_nodes.clone()).await.unwrap();
    }
  }
}
