use {
  crate::{
    consumer::{BlockConsumer, Commitment},
    vm::{Executed, Transaction},
  },
  tracing::info,
};

/// This type is used to sync updates to the blockchain with an
/// external database. This is used by explorers, analytics, and
/// other systems that need to analyze blockchain data as soon as
/// they become available.
pub struct DatabaseSync;

impl DatabaseSync {
  pub fn new() -> Self {
    Self
  }
}

impl BlockConsumer<Vec<Transaction>> for DatabaseSync {
  fn consume(
    &self,
    block: &Executed<Vec<Transaction>>,
    commitment: Commitment,
  ) {
    info!("dbsync consuming {} at {commitment:?}", block.underlying);
  }
}
