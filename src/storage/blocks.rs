use {
  super::Error,
  crate::{
    consensus::{Block, Produced},
    consumer::{BlockConsumer, Commitment},
    vm::{Executed, ExecutedTransaction, Transaction},
  },
  multihash::Multihash,
  sled::{Db, Tree},
  std::{path::PathBuf, sync::Arc},
};

type BlockType = Vec<Transaction>;

#[derive(Debug)]
pub struct BlockStore {
  db: Arc<Db>,
  detailed: bool,
  history_len: u64,
}

impl BlockStore {
  /// Creates a new block storage.
  /// 
  /// The history len parameter specifies how many latest blocks should be 
  /// kept in storage. This value determines how old is the oldest replayeable
  /// block by this node, and how old is the oldest accessible transaction and
  /// block information through RPC.
  /// 
  /// Detailed means that details of block execution (individual transaction 
  /// logs, tx errors, etc.) are also persisted for [`history_len`] latest blocks.
  /// This value is true when the RPC interface is enabled on the node, otherwise
  /// detailed data is useless for a node to participate in consensus.
  pub fn new(
    directory: PathBuf,
    history_len: u64,
    detailed: bool,
  ) -> Result<Self, Error> {
    let mut directory = directory;
    directory.push("blocks");
    std::fs::create_dir_all(directory.clone())?;

    let db = sled::Config::default()
      .path(directory)
      .use_compression(true)
      .mode(sled::Mode::HighThroughput)
      .open()?;

    Ok(Self {
      detailed,
      db: Arc::new(db),
      history_len,
    })
  }

  /// Returns a block with the highest height at a given commitment.
  pub fn latest(&self, commitment: Commitment) -> Option<Produced<BlockType>> {
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
  ) -> Option<(Executed<BlockType>, Commitment)> {
    let hashes = self.db.open_tree(b"hashes").unwrap();
    hashes.get(&hash.to_bytes()).unwrap().and_then(|height| {
      let height = u64::from_be_bytes(height.as_ref().try_into().unwrap());
      self.get_by_height(height)
    })
  }

  pub fn get_by_height(
    &self,
    height: u64,
  ) -> Option<(Executed<BlockType>, Commitment)> {
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

  pub fn get_block_commitment(&self, height: u64) -> Option<Commitment> {
    let height = height.to_be_bytes();
    let confirmed = self.db.open_tree(b"confirmed").unwrap();
    let finalized = self.db.open_tree(b"finalized").unwrap();

    if confirmed.contains_key(&height).unwrap() {
      Some(Commitment::Confirmed)
    } else if finalized.contains_key(&height).unwrap() {
      Some(Commitment::Finalized)
    } else {
      None
    }
  }

  pub fn get_transaction(
    &self,
    hash: &Multihash,
  ) -> Option<ExecutedTransaction> {
    assert!(self.detailed, "detailed storage is set to false");
    let transactions = self.db.open_tree(b"transactions").unwrap();
    transactions
      .get(hash.to_bytes())
      .unwrap()
      .map(|tx| bincode::deserialize(&tx).unwrap())
  }

  async fn store_raw_block(
    &self,
    block: &Produced<BlockType>,
    commitment: Commitment,
  ) {
    let confirmed = self.db.open_tree(b"confirmed").unwrap();
    let finalized = self.db.open_tree(b"finalized").unwrap();
    let destination = match commitment {
      Commitment::Included => return,
      Commitment::Confirmed => &confirmed,
      Commitment::Finalized => &finalized,
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
      confirmed.remove(block.height.to_be_bytes()).unwrap();
    }

    destination
      .insert(
        block.height.to_be_bytes(), // big endian for lexographic byte order
        bincode::serialize(block).unwrap(),
      )
      .unwrap();
  }

  /// Stores the results of executed transactions in a block.
  async fn store_outputs(&self, block: &Executed<BlockType>) {
    assert!(self.detailed, "detailed storage is set to false");
    let block = block.clone();
    let outputs = self.db.open_tree(b"outputs").unwrap();
    let transactions = self.db.open_tree(b"transactions").unwrap();

    let heightkey = block.height.to_be_bytes();
    if !outputs.contains_key(&heightkey).unwrap() {
      outputs
        .insert(
          heightkey,
          bincode::serialize(block.output.as_ref()).unwrap(),
        )
        .unwrap();
    }

    // store all individual transactions with their outputs
    let mut txbatch = sled::Batch::default();
    for tx in block.underlying.data.iter() {
      let txhash = tx.hash();
      let logs = block.output.logs.get(txhash);
      let error = block.output.errors.get(txhash);
      let tx = ExecutedTransaction {
        block: block.height,
        transaction: tx.clone(),
        output: error
          .map(|e| Err(e.clone()))
          .or_else(|| logs.map(|l| Ok(l.clone())))
          .unwrap(),
      };
      let txbytes = bincode::serialize(&tx).unwrap().to_vec();
      txbatch.insert(txhash.to_bytes(), txbytes);
    }

    transactions.apply_batch(txbatch).unwrap();
  }

  /// Removes blocks and all associated outputs, transactions and logs
  /// belonging to those blocks for all committment levels from the storage,
  /// that have block heights lower than then given height.
  async fn prune_older_than(&self, height: u64) {
    if height > 0 {
      let hashes = self.db.open_tree(b"hashes").unwrap();
      let outputs = self.db.open_tree(b"outputs").unwrap();
      let confirmed = self.db.open_tree(b"confirmed").unwrap();
      let finalized = self.db.open_tree(b"finalized").unwrap();
      let transactions = self.db.open_tree(b"transactions").unwrap();

      let prune_tree = |tree: Tree, height: u64, isblock: bool| {
        let zero = 0u64.to_be_bytes();
        let height = height.to_be_bytes();
        let mut drain = tree.range(zero..height);
        while let Some(Ok((h, b))) = drain.next() {
          tree.remove(&h).unwrap();
          if isblock {
            let deserialized: Produced<BlockType> =
              bincode::deserialize(&b).unwrap();
            hashes
              .remove(deserialized.hash().unwrap().to_bytes())
              .unwrap();

            // remove all transactions associated with the
            // pruned block.
            if self.detailed {
              let mut txbatch = sled::Batch::default();
              for tx in deserialized.data.iter() {
                txbatch.remove(tx.hash().to_bytes());
              }
              transactions.apply_batch(txbatch).unwrap();
            }
          }
        }
      };

      if self.detailed {
        tokio::join!(
          async { prune_tree(outputs, height, false) },
          async { prune_tree(confirmed, height, true) },
          async { prune_tree(finalized, height, true) }
        );
      } else {
        tokio::join!(
          async { prune_tree(confirmed, height, true) }, //
          async { prune_tree(finalized, height, true) }
        );
      }
    }
  }
}

impl Clone for BlockStore {
  fn clone(&self) -> Self {
    Self {
      detailed: self.detailed,
      db: Arc::clone(&self.db),
      history_len: self.history_len,
    }
  }
}

#[async_trait::async_trait]
impl BlockConsumer<BlockType> for BlockStore {
  /// The block consumer guarantees that we will get all blocks in order
  /// and without gaps, their height should be monotonically increasing.
  async fn consume(&self, block: Executed<BlockType>, commitment: Commitment) {
    if commitment == Commitment::Included {
      return; // unconfirmed blocks are not persisted
    }

    if self.detailed {
      tokio::join!(
        // Store the block itself, as it was transmitted over the wire
        self.store_raw_block(block.underlying.as_ref(), commitment),
        // Store transactions outputs for this block
        self.store_outputs(&block),
        // remove old blocks that are older than the history limit.
        self.prune_older_than(block.height.saturating_sub(self.history_len))
      );
    } else {
      tokio::join!(
        // Store the block itself, as it was transmitted over the wire
        self.store_raw_block(block.underlying.as_ref(), commitment),
        // remove old blocks that are older than the history limit.
        self.prune_older_than(block.height.saturating_sub(self.history_len))
      );
    }

    // store a mapping of block_hash -> height for
    // fast lookup by blockid
    let hashes = self.db.open_tree(b"hashes").unwrap();
    let hashkey = block.hash().unwrap().to_bytes();
    if !hashes.contains_key(&hashkey).unwrap() {
      hashes
        .insert(hashkey, block.height.to_be_bytes().as_ref())
        .unwrap();
    }
  }
}
