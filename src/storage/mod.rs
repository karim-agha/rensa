mod blocks;
mod state;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
  #[error("Serialization Error: {0}")]
  Serialization(#[from] bincode::Error),

  #[error("Storage Engine Error: {0}")]
  StorageEngine(#[from] sled::Error),

  #[error("System IO Error: {0}")]
  SystemIO(#[from] std::io::Error),
}

pub use {blocks::BlockStore, state::PersistentState};
