use {
  super::Error,
  crate::{
    consensus::{BlockData, Genesis},
    primitives::{Account, Pubkey},
    vm::{State, StateDiff, StateError},
  },
  rocksdb::{Options, WriteBatch, WriteOptions, DB},
  std::path::PathBuf,
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
  db: DB,
}

impl PersistentState {
  pub fn new<D: BlockData>(
    genesis: &Genesis<D>,
    directory: PathBuf,
  ) -> Result<Self, Error> {
    let mut directory = directory;
    directory.push("state");
    std::fs::create_dir_all(directory.clone())?;

    let mut db_opts = Options::default();
    db_opts.create_if_missing(true);

    let db = DB::open(&db_opts, directory)?;
    for (addr, account) in &genesis.state {
      if db.get(addr).unwrap().is_none() {
        db.put(addr, bincode::serialize(account)?)?
      }
    }

    Ok(Self { db })
  }

  /// Applies a state diff from a finalized block
  pub fn apply(&self, diff: StateDiff) -> Result<(), Error> {
    let mut batch = WriteBatch::default();
    for (addr, account) in diff.into_iter() {
      batch.put(addr.to_vec(), bincode::serialize(&account)?);
    }
    let mut write_opts = WriteOptions::default();
    write_opts.set_sync(true);
    self
      .db
      .write_opt(batch, &write_opts)
      .map_err(Error::StorageEngine)
  }
}

impl State for PersistentState {
  fn get(&self, address: &Pubkey) -> Option<Account> {
    match self.db.get(address) {
      Ok(Some(value)) => Some(bincode::deserialize(&value).unwrap()),
      Ok(None) => None,
      Err(_) => panic!("unrecoverable error while accessing datastore."),
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

  fn hash(&self) -> multihash::Multihash {
    unimplemented!() // not applicable here, having a merkle-tree like mechanism
                     // is too expensive for global state and doesn't fit this
                     // blockchain design. State hashes are only calculated on
                     // state diffs between blocks.
  }
}
