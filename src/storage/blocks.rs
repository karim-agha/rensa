use {
  super::Error,
  crate::{
    consensus::{Block, BlockData, Produced},
    consumer::{BlockConsumer, Commitment},
    vm::Executed,
  },
  multihash::Multihash,
  sled::Db,
  std::{marker::PhantomData, path::PathBuf, sync::Arc},
};

#[derive(Debug)]
pub struct BlockStore<D: BlockData> {
  db: Arc<Db>,
  history_len: u64,
  _marker: PhantomData<D>,
}

impl<D: BlockData> BlockStore<D> {
  pub fn new(directory: PathBuf, history_len: u64) -> Result<Self, Error> {
    let mut directory = directory;
    directory.push("blocks");
    std::fs::create_dir_all(directory.clone())?;

    let db = sled::Config::default()
      .path(directory)
      .use_compression(true)
      .compression_factor(10)
      .mode(sled::Mode::HighThroughput)
      .open()?;

    Ok(Self {
      db: Arc::new(db),
      history_len,
      _marker: PhantomData,
    })
  }

  /// Returns a block with the highest height at a given commitment.
  pub fn latest(&self, commitment: Commitment) -> Option<Produced<D>> {
    let tree = match commitment {
      Commitment::Included => {
        // persistance is only implemented for confirmed+ blocks, that have no
        // forks. In the included stage, there could be many blocks at the same
        // height.
        return None;
      }
      Commitment::Confirmed => self.db.open_tree(b"confirmed").unwrap(),
      Commitment::Finalized => self.db.open_tree(b"finalized").unwrap(),
    };

    tree
      .last()
      .unwrap()
      .and_then(|(_, block)| bincode::deserialize(block.as_ref()).ok())
  }

  /// Tries to get a block with a specific hash
  pub fn get_by_hash(
    &self,
    hash: &Multihash,
  ) -> Option<(Executed<D>, Commitment)> {
    let hashes = self.db.open_tree(b"hashes").unwrap();
    hashes.get(&hash.to_bytes()).unwrap().and_then(|height| {
      let height = u64::from_be_bytes(height.as_ref().try_into().unwrap());
      self.get_by_height(height)
    })
  }

  pub fn get_by_height(
    &self,
    height: u64,
  ) -> Option<(Executed<D>, Commitment)> {
    let height = height.to_be_bytes();
    let confirmed = self.db.open_tree(b"confirmed").unwrap();
    let finalized = self.db.open_tree(b"finalized").unwrap();

    let block = if let Ok(Some(block)) = confirmed.get(&height) {
      Some((bincode::deserialize(&block).unwrap(), Commitment::Confirmed))
    } else if let Ok(Some(block)) = finalized.get(&height) {
      Some((bincode::deserialize(&block).unwrap(), Commitment::Finalized))
    } else {
      None
    };

    if let Some((block, commitment)) = block {
      let outputs = self.db.open_tree(b"outputs").unwrap();
      if let Some(output) = outputs.get(&height).unwrap() {
        let output = bincode::deserialize(&output).unwrap();
        let block = Executed::recreate(block, output);
        return Some((block, commitment));
      }
    }

    None
  }
}

impl<D: BlockData> Clone for BlockStore<D> {
  fn clone(&self) -> Self {
    Self {
      db: Arc::clone(&self.db),
      history_len: self.history_len,
      _marker: PhantomData,
    }
  }
}

impl<D: BlockData> BlockConsumer<D> for BlockStore<D> {
  /// The block consumer guarantees that we will get all blocks in order
  /// and without gaps, their height should be monotonically increasing.
  fn consume(&self, block: &Executed<D>, commitment: Commitment) {
    let tree = match commitment {
      Commitment::Included => return, // unconfirmed blocks are not persisted
      Commitment::Confirmed => self.db.open_tree(b"confirmed").unwrap(),
      Commitment::Finalized => self.db.open_tree(b"finalized").unwrap(),
    };

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

      // if the block being added is finalized, then it was most likely
      // previously inserted as confirmed. Remove it from the confirmed
      // history.
      let confirmed_tree = self.db.open_tree(b"confirmed").unwrap();
      confirmed_tree.remove(block.height.to_be_bytes()).unwrap();
    }

    let outputs = self.db.open_tree(b"outputs").unwrap();
    let heightkey = block.height.to_be_bytes();
    if !outputs.contains_key(&heightkey).unwrap() {
      outputs
        .insert(
          heightkey,
          bincode::serialize(block.output.as_ref()).unwrap(),
        )
        .unwrap();
    }

    tree
      .insert(
        block.height.to_be_bytes(), // big endian for lexographic byte order
        bincode::serialize(block.underlying.as_ref()).unwrap(),
      )
      .unwrap();

    // store a mapping of block_hash -> height for
    // fast lookup by blockid
    let hashes = self.db.open_tree(b"hashes").unwrap();
    let hashkey = block.hash().unwrap().to_bytes();
    if !hashes.contains_key(&hashkey).unwrap() {
      hashes
        .insert(hashkey, block.height.to_be_bytes().as_ref())
        .unwrap();
    }

    // remove old blocks that are older than the history limit.
    let confirmed = self.db.open_tree(b"confirmed").unwrap();
    let finalized = self.db.open_tree(b"finalized").unwrap();
    let cutoff = block.height.saturating_sub(self.history_len);
    if cutoff > 0 {
      let cutoff = cutoff.to_be_bytes();
      let zero = 0u64.to_be_bytes();
      let mut drain = tree.range(zero..cutoff);
      while let Some(Ok((h, b))) = drain.next() {
        confirmed.remove(&h).unwrap();
        finalized.remove(&h).unwrap();
        outputs.remove(&h).unwrap();

        let deserialized: Produced<D> = bincode::deserialize(&b).unwrap();
        hashes
          .remove(deserialized.hash().unwrap().to_bytes())
          .unwrap();
      }
    }
  }
}
