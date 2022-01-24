use super::block;
use crate::primitives::{Keypair, ToBase58String};
use futures::Stream;
use multihash::{Sha3_256, StatefulHasher};
use std::{
  collections::VecDeque,
  pin::Pin,
  task::{Context, Poll},
};
use tracing::{error, info};

pub struct BlockProducer {
  keypair: Keypair,
  pending: VecDeque<block::Produced<String>>,
}

impl BlockProducer {
  pub fn new(keypair: Keypair) -> Self {
    BlockProducer {
      keypair,
      pending: VecDeque::new(),
    }
  }

  pub fn produce(&mut self, slot: u64, prev: &dyn block::Block<String>) {
    let prevhash = prev
      .hash()
      .expect("a block with invalid hash wouldn't make it into the chain");

    info!(
      "producing block at slot {slot} on top of {}",
      prevhash.to_b58()
    );

    match bincode::serialize(prev.data()) {
      Ok(bytes) => {
        // this is a test block producer that
        // just hashes the contents of the previous block.

        let mut sha3 = Sha3_256::default();
        sha3.update(&bytes);
        let data = sha3.finalize();
        let data = bs58::encode(data.as_ref()).into_string();

        self.pending.push_back(
          block::Produced::new(&self.keypair, slot, prevhash, data, vec![])
            .unwrap(),
        );
      }
      Err(e) => error!("Failed to serialize previous block data: {e}"),
    }
  }
}

impl Stream for BlockProducer {
  type Item = block::Produced<String>;

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
