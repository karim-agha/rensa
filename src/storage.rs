use {
  crate::{
    consumer::{BlockConsumer, Commitment},
    vm::{Executed, Transaction},
  },
  tracing::info,
};

pub struct PersitentStorage;

impl PersitentStorage {
  pub fn new() -> Self {
    Self
  }
}

impl BlockConsumer<Vec<Transaction>> for PersitentStorage {
  fn consume(
    &self,
    block: &Executed<Vec<Transaction>>,
    commitment: Commitment,
  ) {
    info!("storage consuming {} at {commitment:?}", block.underlying);
  }
}
