use super::block;
use crate::keys::Keypair;
use flexbuffers::FlexbufferSerializer;
use futures::Stream;
use multihash::{Sha3_256, StatefulHasher};
use serde::Serialize;
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
      bs58::encode(prevhash.to_bytes()).into_string()
    );

    let mut s = FlexbufferSerializer::new();
    match prev.data().serialize(&mut s) {
      Ok(_) => {
        // this is a test block producer that
        // just hashes the contents of the previous block.

        let mut sha3 = Sha3_256::default();
        sha3.update(s.view());
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
