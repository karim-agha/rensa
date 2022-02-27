use {
  super::Error,
  crate::{
    consensus::{Block, BlockData, Produced},
    consumer::{BlockConsumer, Commitment},
    vm::Executed,
  },
  multihash::Multihash,
  rocksdb::{Options, DB},
  std::{marker::PhantomData, path::PathBuf, sync::Arc},
};

#[derive(Debug)]
pub struct BlockStore<D: BlockData> {
  db: Arc<DB>,
  _marker: PhantomData<D>,
}

impl<D: BlockData> BlockStore<D> {
  pub fn new(directory: PathBuf) -> Result<Self, Error> {
    let mut directory = directory;
    directory.push("blocks");
    std::fs::create_dir_all(directory.clone())?;

    let mut dbopts = Options::default();
    dbopts.create_if_missing(true);
    dbopts.create_missing_column_families(true);

    Ok(Self {
      db: Arc::new(DB::open_cf(&dbopts, directory, [
        "confirmed",
        "finalized",
        "hashes",
      ])?),
      _marker: PhantomData,
    })
  }

  /// Returns a block with the highest height at a given commitment.
  pub fn latest(&self, commitment: Commitment) -> Option<Produced<D>> {
    let column_family = match commitment {
      Commitment::Included => {
        // persistance is only implemented for confirmed+ blocks, that have no
        // forks. In the included stage, there could be many blocks at the same
        // height.
        return None;
      }
      Commitment::Confirmed => self.db.cf_handle("confirmed").unwrap(),
      Commitment::Finalized => self.db.cf_handle("finalized").unwrap(),
    };

    let mut iter = self.db.raw_iterator_cf(&column_family);
    iter.seek_to_last();
    iter
      .value()
      .and_then(|bytes| bincode::deserialize(bytes).ok())
  }

  /// Tries to get a block with a specific hash
  pub fn get(
    &self,
    hash: &Multihash,
  ) -> Option<(Produced<D>, Commitment)> {
    if let Ok(Some(height)) = self
      .db
      .get_cf(&self.db.cf_handle("hashes").unwrap(), hash.to_bytes())
    {
      if let Ok(Some(block)) = self
        .db
        .get_cf(&self.db.cf_handle("confirmed").unwrap(), &height)
      {
        return Some((
          bincode::deserialize(&block).unwrap(),
          Commitment::Confirmed,
        ));
      }

      if let Ok(Some(block)) = self
        .db
        .get_cf(&self.db.cf_handle("finalized").unwrap(), &height)
      {
        return Some((
          bincode::deserialize(&block).unwrap(),
          Commitment::Finalized,
        ));
      }
    }

    None
  }
}

impl<D: BlockData> Clone for BlockStore<D> {
  fn clone(&self) -> Self {
    Self {
      db: Arc::clone(&self.db),
      _marker: PhantomData,
    }
  }
}

impl<D: BlockData> BlockConsumer<D> for BlockStore<D> {
  /// The block consumer guarantees that we will get all blocks in order
  /// and without gaps, their height should be monotonically increasing.
  fn consume(&self, block: &Executed<D>, commitment: Commitment) {
    let column_family = match commitment {
      Commitment::Included => return, // unconfirmed blocks are not persisted
      Commitment::Confirmed => self.db.cf_handle("confirmed").unwrap(),
      Commitment::Finalized => self.db.cf_handle("finalized").unwrap(),
    };

    // if the block being added is finalized, then it was most likely previously
    // inserted as a commited. Remove it from the commited history.
    if let Commitment::Finalized = commitment {
      // the finalized state must never have gaps and crash immediately
      // if the inserted block is not an immediate successor to the
      // latest stored finalized block.
      let latest_height =
        self.latest(commitment).map(|b| b.height).unwrap_or(0);
      if block.height != (latest_height + 1) {
        panic!(
          "state persistance inconsistency, latest height is {latest_height}, \
           newly appended block height is {}. Persistent stora cannot have \
           state gaps.",
          block.height
        );
      }
      let confirmed_cf = self.db.cf_handle("confirmed").unwrap();
      self
        .db
        .delete_cf(&confirmed_cf, block.height.to_be_bytes())
        .unwrap();
    }

    self
      .db
      .put_cf(
        &column_family,
        block.height.to_be_bytes(), // big endian for lexographic byte order
        bincode::serialize(&block.underlying).unwrap(),
      )
      .unwrap();

    // store a mapping of block_hash -> height for
    // fast lookup by blockid
    self
      .db
      .put_cf(
        &self.db.cf_handle("hashes").unwrap(),
        block.hash().unwrap().to_bytes(),
        block.height.to_be_bytes(),
      )
      .unwrap();
  }
}
