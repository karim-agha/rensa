use {
  super::Error,
  crate::{
    consensus::{BlockData, Produced},
    consumer::{BlockConsumer, Commitment},
    vm::Executed,
  },
  rocksdb::{Options, DB},
  std::{marker::PhantomData, path::PathBuf},
};

#[derive(Debug)]
pub struct BlockStore<D: BlockData> {
  db: DB,
  _marker: PhantomData<D>,
}

impl<D: BlockData> BlockStore<D> {
  pub fn new(directory: PathBuf) -> Result<Self, Error> {
    let mut directory = directory;
    directory.push("blocks");
    std::fs::create_dir_all(directory.clone())?;

    let mut db_opts = Options::default();
    db_opts.create_if_missing(true);

    Ok(Self {
      db: DB::open(&db_opts, directory)?,
      _marker: PhantomData,
    })
  }

  pub fn latest(&self) -> Option<Produced<D>> {
    let mut iter = self.db.raw_iterator();
    iter.seek_to_last();
    iter
      .value()
      .and_then(|bytes| bincode::deserialize(bytes).ok())
  }
}

impl<D: BlockData> BlockConsumer<D> for BlockStore<D> {
  fn consume(&self, block: &Executed<D>, commitment: Commitment) {
    if let Commitment::Finalized = commitment {
      self
        .db
        .put(
          block.height.to_le_bytes(),
          bincode::serialize(&block.underlying).unwrap(),
        )
        .unwrap();
    }
  }
}
