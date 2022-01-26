use crate::{
  consensus::block,
  primitives::{Keypair, ToBase58String},
  vm::{AccountRef, Transaction},
};
use futures::Stream;
use std::{
  collections::VecDeque,
  pin::Pin,
  task::{Context, Poll},
};
use tracing::info;

pub struct BlockProducer {
  keypair: Keypair,
  pending: VecDeque<block::Produced<Vec<Transaction>>>,
}

impl BlockProducer {
  pub fn new(keypair: Keypair) -> Self {
    BlockProducer {
      keypair,
      pending: VecDeque::new(),
    }
  }

  pub fn produce(
    &mut self,
    slot: u64,
    prev: &dyn block::Block<Vec<Transaction>>,
  ) {
    let prevhash = prev.hash().unwrap();

    // private key of account CKDN1WjimfErkbgecnEfoPfs7CU1TknwMhpgbiXNknGC
    let signer = "9XhCqH1LxmziWmBb8WnqzuvKFjX7koBuyzwdcFkL1ym7"
      .parse()
      .unwrap();

    let tx = Transaction::new(
      "Sha3111111111111111111111111111111111111111"
        .parse()
        .unwrap(),
      vec![AccountRef {
        address: "Test111111111111111111111111111111111111111"
          .parse()
          .unwrap(),
        writable: true,
      }],
      b"initial-seed".to_vec(),
      &[signer],
    );

    let block =
      block::Produced::new(&self.keypair, slot, prevhash, vec![tx], vec![])
        .unwrap();
    info!("{block:#?} on top of {}", prevhash.to_b58());

    self.pending.push_back(block);
  }
}

impl Stream for BlockProducer {
  type Item = block::Produced<Vec<Transaction>>;

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
