use {
  super::Error,
  crate::{
    consensus::{BlockData, Genesis},
    primitives::{Account, Pubkey},
    vm::{State, StateDiff, StateError},
  },
  sled::{Batch, Db},
  std::{path::PathBuf, sync::Arc},
};

/// This type represents a storage that is persisted on disk and survives node
/// crashes and restarts. It is used for state that is finelized and is
/// guaranteed to never be reverted.
///
/// Volatile state that is still subject to consensus voting lives in the
/// ['forktree`] module in consensus.
///
/// The persistent state is a key-value map of account addresses to their
/// account values. The byte representation of the account value is serialized
/// and deserialized using [`bincode`] serializer + serde.
///
/// The blockchain state lives in the default column family.
/// The database uses other column families for other types of data,
/// such as the contents of recent blocks, etc, but the default
/// Column Family is used for the accounts store.
#[derive(Debug)]
pub struct PersistentState {
  db: Arc<Db>,
}

impl PersistentState {
  pub fn new<D: BlockData>(
    genesis: &Genesis<D>,
    directory: PathBuf,
  ) -> Result<Self, Error> {
    let mut directory = directory;
    directory.push("state");
    std::fs::create_dir_all(directory.clone())?;

    let db = sled::open(directory)?;
    if db.is_empty() {
      for (addr, account) in &genesis.state {
        if db.get(addr).unwrap().is_none() {
          db.insert(addr, bincode::serialize(account)?)?;
        }
      }
    }

    Ok(Self { db: Arc::new(db) })
  }

  /// Applies a state diff from a finalized block
  pub fn apply(&self, diff: &StateDiff) -> Result<(), Error> {
    let mut batch = Batch::default();
    for (addr, account) in diff.iter() {
      match account {
        Some(account) => {
          batch.insert(addr.as_ref(), bincode::serialize(&account)?)
        }
        None => batch.remove(addr.as_ref()),
      };
    }
    self.db.apply_batch(batch).map_err(Error::StorageEngine)
  }
}

impl State for PersistentState {
  fn get(&self, address: &Pubkey) -> Option<Account> {
    match self.db.get(address) {
      Ok(Some(value)) => Some(bincode::deserialize(&value).unwrap()),
      Ok(None) => None,
      Err(e) => panic!("unrecoverable error while accessing datastore: {e:?}"),
    }
  }

  /// Writes directly to finalized state are not supported, instead
  /// state diffs from newly finalized blocks should be applied using the
  /// [`apply`] method
  fn set(
    &mut self,
    _address: Pubkey,
    _account: Account,
  ) -> Result<Option<Account>, StateError> {
    Err(StateError::WritesNotSupported)
  }

  /// Writes directly to finalized state are not supported, instead
  /// state diffs from newly finalized blocks should be applied using the
  /// [`apply`] method
  fn remove(&mut self, _address: Pubkey) -> Result<(), StateError> {
    Err(StateError::WritesNotSupported)
  }

  fn hash(&self) -> multihash::Multihash {
    unimplemented!() // not applicable here, having a merkle-tree like mechanism
                     // is too expensive for global state and doesn't fit this
                     // blockchain design. State hashes are only calculated on
                     // state diffs between blocks.
  }
}

impl Clone for PersistentState {
  fn clone(&self) -> Self {
    Self {
      db: Arc::clone(&self.db),
    }
  }
}
