//! THIS IS VERY MUCH WORK IN PROGRESS, USED ONLY FOR TESTING NOW,
//! AND SHOULD BE REWRITTEN FROM SCRATCH VERY SOON.

use {
  crate::{
    consensus::{Block, Genesis, Produced, Vote},
    consumer::{BlockConsumer, Commitment},
    primitives::{Keypair, Pubkey, ToBase58String},
    vm::{self, AccountRef, Executable, State, Transaction},
  },
  borsh::BorshSerialize,
  dashmap::{DashMap, DashSet},
  futures::Stream,
  multihash::Multihash,
  rand::{distributions::Uniform, thread_rng, Rng},
  rayon::prelude::*,
  std::{
    pin::Pin,
    sync::{
      atomic::{AtomicBool, Ordering},
      Arc,
    },
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

  pub fn take_transactions(&self) -> Vec<Transaction> {
    let output = self.txs.iter().map(|t| t.value().clone()).collect();
    self.txs.clear();
    output
  }
}

/// used only during early development to simulate lots of txs
/// Should be moved to a separate module asap that sends those
/// txs through rpc
struct TestScenario {
  keypair: Keypair,
  payer: Keypair,
  mint: Pubkey,
  alternate: AtomicBool,
  wallets_a: Vec<Keypair>,
  wallets_b: Vec<Keypair>,
}

impl TestScenario {
  pub fn new(keypair: &Keypair) -> Self {
    let currency_addr: Pubkey = "Currency1xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
      .parse()
      .unwrap();
    let mint_addr = currency_addr.derive(&[&keypair.public()]);
    Self {
      keypair: keypair.clone(),
      payer: "6MiU5w4RZVvCDqvmitDqFdU5QMoeS7ywA6cAnSeEFdW"
        .parse()
        .unwrap(),
      mint: mint_addr,
      alternate: AtomicBool::new(false),
      wallets_a: (0..2500)
        .into_par_iter()
        .map(|_| Keypair::unique())
        .collect(),
      wallets_b: (0..2500)
        .into_par_iter()
        .map(|_| Keypair::unique())
        .collect(),
    }
  }

  pub fn transactions(&self, state: &dyn State) -> Vec<Transaction> {
    let alternate = self.alternate.fetch_xor(true, Ordering::SeqCst);
    let txs = if state.get(&self.mint).is_none() {
      let seed = self.keypair.public().to_vec();
      self.create_mint_txs(&self.payer, seed.try_into().unwrap())
    } else if alternate {
      self
        .wallets_a
        .par_iter()
        .zip(self.wallets_b.par_iter())
        .map(|(from, to)| {
          Self::create_transfer_tx(&self.payer, &self.mint, from, &to.public())
        })
        .collect()
    } else {
      self
        .wallets_b
        .par_iter()
        .zip(self.wallets_a.par_iter())
        .map(|(from, to)| {
          Self::create_transfer_tx(&self.payer, &self.mint, from, &to.public())
        })
        .collect()
    };

    txs
  }

  fn create_transfer_tx(
    payer: &Keypair,
    mint: &Pubkey,
    from: &Keypair,
    to: &Pubkey,
  ) -> Transaction {
    let currency_addr: Pubkey = "Currency1xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
      .parse()
      .unwrap();

    let from_coin_addr = currency_addr.derive(&[mint, &from.public()]);
    let to_coin_addr = currency_addr.derive(&[mint, to]);

    let dist = Uniform::new(90000, 110000);
    let amount = thread_rng().sample(dist);
    Transaction::new(
      currency_addr,
      payer,
      vec![
        AccountRef::readonly(*mint, false).unwrap(),
        AccountRef::readonly(from.public(), true).unwrap(),
        AccountRef::writable(from_coin_addr, false).unwrap(),
        AccountRef::readonly(*to, false).unwrap(),
        AccountRef::writable(to_coin_addr, false).unwrap(),
      ],
      vm::builtin::currency::Instruction::Transfer(amount)
        .try_to_vec()
        .unwrap(),
      &[from],
    )
  }

  /// Creates the coin and then mints 10k coins to
  /// each wallet in group a and group b
  fn create_mint_txs(
    &self,
    payer: &Keypair,
    seed: [u8; 32],
  ) -> Vec<Transaction> {
    let currency_addr: Pubkey = "Currency1xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
      .parse()
      .unwrap();

    let mint_addr = currency_addr.derive(&[&seed]);
    let create_tx = Transaction::new(
      currency_addr,
      payer,
      vec![AccountRef::writable(mint_addr, false).unwrap()],
      vm::builtin::currency::Instruction::Create {
        seed,
        authority: self.keypair.public(),
        decimals: 2,
        name: None,
        symbol: None,
      }
      .try_to_vec()
      .unwrap(),
      &[&self.keypair],
    );

    let mut txs = Vec::with_capacity(5001);

    // create the coin
    txs.push(create_tx);

    // mint 10k coins for each account
    txs.append(
      &mut self
        .wallets_a
        .par_iter()
        .chain(self.wallets_b.par_iter())
        .map(|w| {
          Transaction::new(
            currency_addr,
            payer,
            vec![
              AccountRef::writable(mint_addr, false).unwrap(),
              AccountRef::readonly(self.keypair.public(), true).unwrap(),
              AccountRef::readonly(w.public(), false).unwrap(),
              AccountRef::writable(
                currency_addr.derive(&[&mint_addr, &w.public()]),
                false,
              )
              .unwrap(),
            ],
            vm::builtin::currency::Instruction::Mint(1000000)
              .try_to_vec()
              .unwrap(),
            &[&self.keypair],
          )
        })
        .collect(),
    );

    txs
  }
}

pub struct BlockProducer {
  keypair: Keypair,
  mempool: Arc<MempoolState>,
  tester: Arc<TestScenario>,
  pending: Option<Produced<Vec<Transaction>>>,
}

impl Clone for BlockProducer {
  fn clone(&self) -> Self {
    Self {
      keypair: self.keypair.clone(),
      mempool: Arc::clone(&self.mempool),
      tester: Arc::clone(&self.tester),
      pending: None,
    }
  }
}

impl BlockProducer {
  pub fn new(genesis: &Genesis<Vec<Transaction>>, keypair: Keypair) -> Self {
    BlockProducer {
      keypair: keypair.clone(),
      tester: Arc::new(TestScenario::new(&keypair)),
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

    let mut txs = self.tester.transactions(state);

    let votes = self.mempool.take_votes();
    let mut mempool = self.mempool.take_transactions();
    txs.append(&mut mempool);

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

  pub fn record_vote(&self, vote: Vote) {
    self.mempool.add_vote(vote);
  }

  pub fn record_transaction(&self, transaction: Transaction) {
    self.mempool.add_transaction(transaction);
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
impl BlockConsumer<Vec<Transaction>> for BlockProducer {
  fn consume(
    &self,
    block: &vm::Executed<Vec<Transaction>>,
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
