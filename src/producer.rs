use std::{
  collections::{HashMap, HashSet, VecDeque},
  mem::take,
  pin::Pin,
  task::{Context, Poll},
};

use futures::Stream;
use tracing::info;

use crate::{
  consensus::{Block, Genesis, Produced, Vote},
  primitives::{Keypair, Pubkey, ToBase58String},
  vm::{AccountRef, Transaction},
};

pub struct BlockProducer {
  keypair: Keypair,
  votes: HashMap<[u8; 64], Vote>,
  validators: HashSet<Pubkey>,
  pending: VecDeque<Produced<Vec<Transaction>>>,
}

impl BlockProducer {
  pub fn new(genesis: &Genesis<Vec<Transaction>>, keypair: Keypair) -> Self {
    BlockProducer {
      keypair,
      votes: HashMap::new(),
      validators: genesis
        .validators
        .iter()
        .map(|v| v.pubkey.clone())
        .collect(),
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

  pub fn produce(&mut self, slot: u64, prev: &dyn Block<Vec<Transaction>>) {
    let prevhash = prev.hash().unwrap();

    let payer = "6MiU5w4RZVvCDqvmitDqFdU5QMoeS7ywA6cAnSeEFdW"
      .parse()
      .unwrap();

    // private key of account CKDN1WjimfErkbgecnEfoPfs7CU1TknwMhpgbiXNknGC
    let signer = "9XhCqH1LxmziWmBb8WnqzuvKFjX7koBuyzwdcFkL1ym7"
      .parse()
      .unwrap();

    let tx = Transaction::new(
      "Sha3111111111111111111111111111111111111111"
        .parse()
        .unwrap(),
      &payer,
      vec![AccountRef {
        address: "CKDN1WjimfErkbgecnEfoPfs7CU1TknwMhpgbiXNknGC"
          .parse()
          .unwrap(),
        writable: true,
      }],
      b"initial-seed".to_vec(),
      &[&signer],
    );

    // let votes = take(&mut self.votes);

    let block = Produced::new(
      &self.keypair,
      slot,
      prevhash,
      vec![tx; 5000],
      take(&mut self.votes).into_iter().map(|(_, v)| v).collect(),
    )
    .unwrap();
    info!(
      "Produced {block} on top of {} with {} transactions",
      prevhash.to_b58(),
      block.data.len()
    );
    self.pending.push_back(block);
  }
}

impl Stream for BlockProducer {
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
