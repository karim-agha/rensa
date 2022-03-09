use {
  super::{contract::ContractError, State, StateDiff},
  crate::primitives::ToBase58String,
  multihash::{
    Code as MultihashCode,
    Hasher,
    Multihash,
    MultihashDigest,
    Sha3_256,
  },
  once_cell::sync::OnceCell,
  std::collections::BTreeMap,
};

/// mapping tx_hash -> error that caused the tx to fail
#[derive(Default)]
pub struct ErrorsMap {
  inner: BTreeMap<Multihash, ContractError>,
  hashcache: OnceCell<Multihash>,
}

impl ErrorsMap {
  pub fn new() -> Self {
    Self {
      inner: BTreeMap::new(),
      hashcache: OnceCell::new(),
    }
  }

  pub fn hash(&self) -> &Multihash {
    self.hashcache.get_or_init(|| {
      let mut hasher = Sha3_256::default();
      for (k, v) in self.inner.iter() {
        hasher.update(&k.to_bytes());
        hasher.update(v.to_string().as_bytes());
      }
      MultihashCode::Sha3_256.wrap(hasher.finalize()).unwrap()
    })
  }
}

impl std::ops::Deref for ErrorsMap {
  type Target = BTreeMap<Multihash, ContractError>;

  fn deref(&self) -> &Self::Target {
    &self.inner
  }
}

impl std::ops::DerefMut for ErrorsMap {
  fn deref_mut(&mut self) -> &mut Self::Target {
    &mut self.inner
  }
}

/// mapping tx_hash -> list of logs generated by tx
#[derive(Default)]
pub struct LogsMap {
  inner: BTreeMap<Multihash, Vec<(String, String)>>,
  hashcache: OnceCell<Multihash>,
}

impl LogsMap {
  pub fn new() -> Self {
    Self {
      inner: BTreeMap::new(),
      hashcache: OnceCell::new(),
    }
  }

  pub fn hash(&self) -> &Multihash {
    self.hashcache.get_or_init(|| {
      let mut hasher = Sha3_256::default();
      for (k, v) in self.inner.iter() {
        hasher.update(&k.to_bytes());
        for (lk, lv) in v.iter() {
          hasher.update(lk.as_bytes());
          hasher.update(lv.as_bytes());
        }
      }
      MultihashCode::Sha3_256.wrap(hasher.finalize()).unwrap()
    })
  }
}

impl std::ops::Deref for LogsMap {
  type Target = BTreeMap<Multihash, Vec<(String, String)>>;

  fn deref(&self) -> &Self::Target {
    &self.inner
  }
}

impl std::ops::DerefMut for LogsMap {
  fn deref_mut(&mut self) -> &mut Self::Target {
    &mut self.inner
  }
}

/// The result of executing a transaction.
#[derive(Default)]
pub struct TransactionOutput {
  /// The resulting changes to the blockchain global state.
  pub state_diff: StateDiff,

  /// The generated log entries for explorers and chain clients.
  pub log_entries: Vec<(String, String)>,
}

impl TransactionOutput {
  pub fn merge(self, newer: TransactionOutput) -> TransactionOutput {
    let mut newer = newer;
    let mut state = self.state_diff;
    let mut logs = self.log_entries;

    state = state.merge(newer.state_diff);
    logs.append(&mut newer.log_entries);

    Self {
      state_diff: state,
      log_entries: logs,
    }
  }
}

/// This struct represents the result of executing all
/// transactions within a block.
#[derive(Default)]
pub struct BlockOutput {
  /// Changes to the global ledger state.
  pub state: StateDiff,

  /// Client-facing logs and receipts generated by a transaction
  pub logs: LogsMap,

  /// Failed transactions and their failure error message
  pub errors: ErrorsMap,

  /// Hash of the state logs of transactions
  hashcache: OnceCell<Multihash>,
}

impl BlockOutput {
  pub fn new(state: StateDiff, logs: LogsMap, errors: ErrorsMap) -> Self {
    Self {
      state,
      logs,
      errors,
      hashcache: OnceCell::new(),
    }
  }

  pub fn hash(&self) -> &Multihash {
    self.hashcache.get_or_init(|| {
      let mut hasher = Sha3_256::default();
      hasher.update(&self.state.hash().to_bytes());
      hasher.update(&self.logs.hash().to_bytes());
      hasher.update(&self.errors.hash().to_bytes());
      MultihashCode::Sha3_256.wrap(hasher.finalize()).unwrap()
    })
  }
}

impl std::fmt::Debug for BlockOutput {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("BlockOutput")
      .field("state", &self.state.hash().to_bytes().to_b58())
      .field("logs", &self.logs.hash().to_bytes().to_b58())
      .field("errors", &self.errors.hash().to_bytes().to_b58())
      .finish()
  }
}