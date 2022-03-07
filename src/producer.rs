use {
  crate::{
    consensus::{Block, Genesis, Produced, Vote},
    primitives::{Keypair, Pubkey, ToBase58String},
    vm::{self, AccountRef, Executable, State, Transaction},
  },
  futures::Stream,
  rayon::prelude::*,
  std::{
    collections::{HashMap, HashSet, VecDeque},
    mem::take,
    pin::Pin,
    task::{Context, Poll},
  },
  tracing::info,
};

pub struct BlockProducer<'v> {
  keypair: Keypair,
  vm: &'v vm::Machine,
  votes: HashMap<[u8; 64], Vote>,
  validators: HashSet<Pubkey>,
  pending: VecDeque<Produced<Vec<Transaction>>>,
}

impl<'v> BlockProducer<'v> {
  pub fn new(
    genesis: &Genesis<Vec<Transaction>>,
    vm: &'v vm::Machine,
    keypair: Keypair,
  ) -> Self {
    BlockProducer {
      vm,
      keypair,
      votes: HashMap::new(),
      validators: genesis.validators.iter().map(|v| v.pubkey).collect(),
      pending: VecDeque::new(),
    }
  }

  pub fn record_vote(&mut self, vote: Vote) {
    // todo: use BLS aggregate signature to save space and bandwidth
    if self.validators.contains(&vote.validator) {
      self.votes.insert(vote.signature.to_bytes(), vote);
    }
  }

  // remove votes that were already observed in received blocks.
  pub fn exclude_votes(&mut self, block: &Produced<Vec<Transaction>>) {
    for vote in &block.votes {
      self.votes.remove(&vote.signature.to_bytes());
    }
  }

  fn create_sha_tx(payer: &Keypair) -> Transaction {
    // private key of account CKDN1WjimfErkbgecnEfoPfs7CU1TknwMhpgbiXNknGC
    let signer = "9XhCqH1LxmziWmBb8WnqzuvKFjX7koBuyzwdcFkL1ym7"
      .parse()
      .unwrap();

    Transaction::new(
      "Sha3xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
        .parse()
        .unwrap(),
      payer,
      vec![AccountRef::writable(
        "CKDN1WjimfErkbgecnEfoPfs7CU1TknwMhpgbiXNknGC",
        true,
      )
      .unwrap()],
      b"initial-seed".to_vec(),
      &[&signer],
    )
  }

  pub fn produce(
    &mut self,
    state: &dyn State,
    prev: &dyn Block<Vec<Transaction>>,
  ) {
    let prevhash = prev.hash().unwrap();

    // this account pays the tx costs
    let payer = "6MiU5w4RZVvCDqvmitDqFdU5QMoeS7ywA6cAnSeEFdW"
      .parse()
      .unwrap();

    let tx = Self::create_sha_tx(&payer);
    let txs = vec![tx; 5000];
    let statediff = txs.execute(self.vm, state).unwrap();
    let state_hash = statediff.hash();

    let block = Produced::new(
      &self.keypair,
      prev.height() + 1,
      prevhash,
      txs,
      state_hash,
      take(&mut self.votes).into_iter().map(|(_, v)| v).collect(),
    )
    .unwrap();
    info!(
      "Produced {block} on top of {} with {} transactions with state hash: {}",
      prevhash.to_b58(),
      block.data.len(),
      state_hash.to_b58()
    );
    self.pending.push_back(block);
  }
}

/// Generates a fixed number of user wallet keypair
/// Used for test scenarios only.
fn _generate_wallets(count: usize) -> Vec<Keypair> {
  use rand::{prelude::ThreadRng, RngCore};
  (0..count)
    .into_par_iter()
    .filter_map(|_| {
      let mut secret = [0u8; 32];
      ThreadRng::default().fill_bytes(&mut secret);
      (&secret[..]).try_into().ok()
    })
    .collect()
}

impl<'v> Stream for BlockProducer<'v> {
  type Item = Produced<Vec<Transaction>>;

  fn poll_next(
    mut self: Pin<&mut Self>,
    _: &mut Context<'_>,
  ) -> Poll<Option<Self::Item>> {
    if let Some(block) = self.pending.pop_front() {
      return Poll::Ready(Some(block));
    }
    Poll::Pending
  }
}
