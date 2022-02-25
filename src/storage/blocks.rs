use {
  super::Error,
  crate::{
    consensus::{BlockData, Produced},
    consumer::{BlockConsumer, Commitment},
    vm::Executed,
  },
  rocksdb::DB,
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

    Ok(Self {
      db: Arc::new(DB::open_default(directory)?),
      _marker: PhantomData,
    })
  }

  pub fn latest(&self, commitment: Commitment) -> Option<Produced<D>> {
    if let Commitment::Finalized = commitment {
      let mut iter = self.db.raw_iterator();
      iter.seek_to_last();
      iter
        .value()
        .and_then(|bytes| bincode::deserialize(bytes).ok())
    } else {
      None // todo
    }
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
  fn consume(&self, block: &Executed<D>, commitment: Commitment) {
    if let Commitment::Finalized = commitment {
      self
        .db
        .put(
          block.height.to_be_bytes(), // big endian for lexographic byte order
          bincode::serialize(&block.underlying).unwrap(),
        )
        .unwrap();
    }
  }
}
