use {
  super::Error,
  crate::{
    consensus::{BlockData, Produced},
    consumer::{BlockConsumer, Commitment},
    vm::Executed,
  },
  rocksdb::{FlushOptions, Options, DB},
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

    let mut db_opts = Options::default();
    db_opts.create_if_missing(true);
    db_opts.set_use_fsync(true);
    db_opts.set_use_direct_reads(true);
    db_opts.set_use_direct_io_for_flush_and_compaction(true);

    Ok(Self {
      db: Arc::new(DB::open(&db_opts, directory)?),
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
          block.height.to_ne_bytes(),
          bincode::serialize(&block.underlying).unwrap(),
        )
        .unwrap();

      let mut flushopts = FlushOptions::new();
      flushopts.set_wait(true);
      self.db.flush_opt(&flushopts).unwrap();
      self.db.flush_wal(true).unwrap();
    }
  }
}
