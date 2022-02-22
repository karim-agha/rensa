use {
  crate::{
    consumer::{BlockConsumer, Commitment},
    vm::{Executed, Transaction},
  },
  rayon::prelude::*,
  rocksdb::DB,
  std::path::PathBuf,
  tracing::info,
};

pub struct PersitentStorage {
  db: DB,
}

impl PersitentStorage {
  pub fn new(directory: PathBuf) -> Result<Self, rocksdb::Error> {
    Ok(Self {
      db: DB::open_default(directory)?,
    })
  }
}

impl BlockConsumer<Vec<Transaction>> for PersitentStorage {
  fn consume(
    &self,
    block: &Executed<Vec<Transaction>>,
    commitment: Commitment,
  ) {
    if let Commitment::Finalized = commitment {
      info!("storage consuming {} at {commitment:?}", block.underlying);
      // store the resulting state
      block.state_diff.clone().into_iter().par_bridge().for_each(
        |(addr, account)| {
          self
            .db
            .put(addr.to_vec(), bincode::serialize(&account).unwrap())
            .unwrap()
        },
      );
    }
  }
}
