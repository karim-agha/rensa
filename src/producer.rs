use {
  crate::{
    consensus::{Block, Genesis, Limits, Produced, Vote},
    consumer::{BlockConsumer, Commitment},
    primitives::{Keypair, Pubkey, ToBase58String},
    vm::{self, Executable, State, Transaction},
  },
  dashmap::{DashMap, DashSet},
  futures::Stream,
  itertools::Itertools,
  multihash::Multihash,
  std::{
    collections::BTreeMap,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
  },
  tracing::{debug, info},
};

struct MempoolState {
  validators: DashSet<Pubkey>,
  votes: DashMap<[u8; 64], Vote>,
  txs: DashMap<Multihash, Transaction>,
}

impl MempoolState {
  pub fn new(validators: DashSet<Pubkey>) -> Self {
    Self {
      votes: DashMap::new(),
      txs: DashMap::new(),
      validators,
    }
  }

  pub fn add_vote(&self, vote: Vote) {
    // todo: use BLS aggregate signature to save space and bandwidth
    if self.validators.contains(&vote.validator) {
      self.votes.insert(vote.signature.to_bytes(), vote);
    }
  }

  pub fn add_transaction(&self, transaction: Transaction) {
    if transaction.verify_signatures().is_ok() {
      debug!("adding transaction {transaction} to mempool");
      self.txs.insert(*transaction.hash(), transaction);
    }
  }

  pub fn take_votes(&self) -> Vec<Vote> {
    let output = self.votes.iter().map(|v| v.value().clone()).collect();
    self.votes.clear();
    output
  }

  /// Moves out a number of transactions from the mempool.
  ///
  /// Here few things happen:
  ///   1. All output transactions are ordered by the transaction hash,
  ///      to minimize the impact of MEV and make the transaction ordering
  ///      less predictable, to prevent frontrunning or sandwitch attacks.
  ///   2. Transactions belonging to the same payer, are ordered ascendingly
  ///      by the nonce value, to allow multiple transactions from the same
  ///      payer within one block.
  ///   3. per #3 the ordering by the transaction hash, excludes transactions
  ///      issued by the same payer, and in that case only the first transaction
  ///      hash in the sequence of increasing nonces is used during sorting by
  /// hash.
  pub fn take_transactions(&self, count: usize) -> Vec<Transaction> {
    // keep all transactions sorted by ther hashes (MEV mitigation)
    let mut output = BTreeMap::new();

    // get all transactions groupped by their payer, to allow multiple
    // transactions from the same payer within the same block, and insert them
    // into a sorted list by the transaction hash, or the hash of the first
    // transaction for a group of transaction by the same payer.
    for (_, txs) in self.txs.iter().group_by(|t| t.payer).into_iter() {
      let mut txs: Vec<_> = txs.collect(); // order by nonce for the same payer
      txs.sort_by(|a, b| a.nonce.cmp(&b.nonce));
      if let Some(first) = txs.first() {
        output.insert(*first.hash(), txs);
      }
    }

    // flatten the sorted transactions list
    // and consume up to "count" transactions
    let output: Vec<_> = output
      .into_iter()
      .flat_map(|(_, tx)| tx.into_iter())
      .map(|kv| kv.value().clone())
      .take(count)
      .collect();

    // remove from mempool the selected txs
    output.iter().for_each(|tx| {
      self.txs.remove(tx.hash());
    });

    output
  }
}

/// This type is responsible for maintaining a list of transactions
/// that were submitted through RPC to one of the validators then
/// gossiped to the network, and producing new blocks when it is this
/// validator's turn to produce one.
///
/// It also implements the BlockConsumer interface so that it can remove
/// pending transactions that already appeared in blocks produced by
/// other validators.
pub struct BlockProducer {
  keypair: Keypair,
  limits: Limits,
  mempool: Arc<MempoolState>,
  pending: Option<Produced<Vec<Transaction>>>,
}

impl Clone for BlockProducer {
  fn clone(&self) -> Self {
    Self {
      keypair: self.keypair.clone(),
      mempool: Arc::clone(&self.mempool),
      limits: self.limits.clone(),
      pending: None,
    }
  }
}

impl BlockProducer {
  pub fn new(genesis: &Genesis<Vec<Transaction>>, keypair: Keypair) -> Self {
    BlockProducer {
      keypair,
      limits: genesis.limits.clone(),
      mempool: Arc::new(MempoolState::new(
        genesis.validators.iter().map(|v| v.pubkey).collect(),
      )),
      pending: None,
    }
  }

  pub fn produce(
    &mut self,
    state: &dyn State,
    prev: &dyn Block<Vec<Transaction>>,
    vm: &vm::Machine,
  ) {
    let prevheight = prev.height();
    let prevhash = prev.hash().unwrap();

    let votes = self.mempool.take_votes();
    let txs = self
      .mempool
      .take_transactions(self.limits.max_block_transactions);

    let blockoutput = txs.execute(vm, state).unwrap();
    let state_hash = blockoutput.hash();
    let block = Produced::new(
      &self.keypair,
      prevheight + 1,
      prevhash,
      txs,
      *state_hash,
      votes,
    )
    .unwrap();
    info!(
      "Produced {block} on top of {} with {} transactions with state hash: {}",
      prevhash.to_b58(),
      block.data.len(),
      state_hash.to_b58()
    );
    self.pending = Some(block);
  }

  pub fn reuse_discarded(&self, block: Produced<Vec<Transaction>>) {
    // try to reuse votes
    for vote in block.votes {
      self.record_vote(vote);
    }

    // try to reinclude transactions
    for tx in block.data {
      self.record_transaction(tx);
    }
  }

  pub fn record_vote(&self, vote: Vote) {
    self.mempool.add_vote(vote);
  }

  pub fn record_transaction(&self, transaction: Transaction) {
    if transaction.verify_limits(&self.limits).is_ok() {
      self.mempool.add_transaction(transaction);
    }
  }
}

impl Stream for BlockProducer {
  type Item = Produced<Vec<Transaction>>;

  fn poll_next(
    mut self: Pin<&mut Self>,
    _: &mut Context<'_>,
  ) -> Poll<Option<Self::Item>> {
    if let Some(block) = self.pending.take() {
      return Poll::Ready(Some(block));
    }
    Poll::Pending
  }
}

/// Exclude already included blocks and votes on a background thread
#[async_trait::async_trait]
impl BlockConsumer<Vec<Transaction>> for BlockProducer {
  async fn consume(
    &self,
    block: Arc<vm::Executed<Vec<Transaction>>>,
    commitment: Commitment,
  ) {
    if let Commitment::Included = commitment {
      // don't duplicate votes if they were
      // already included by an accepted block.
      for vote in block.votes() {
        self.mempool.votes.remove(&vote.signature.to_bytes());
      }

      // remove transactions from the mempool if they were
      // already included by an accepted block.
      for tx in &block.data {
        self.mempool.txs.remove(tx.hash());
      }
    }
  }
}
