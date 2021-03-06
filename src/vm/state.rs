use {
  super::Executed,
  crate::{
    consensus::{Block, BlockData},
    primitives::{Account, Pubkey},
    storage::Error as StorageError,
  },
  multihash::{
    Code as MultihashCode,
    Hasher,
    Multihash,
    MultihashDigest,
    Sha3_256,
  },
  once_cell::sync::OnceCell,
  serde::{Deserialize, Serialize},
  std::{
    collections::{BTreeMap, BTreeSet},
    ops::Deref,
    sync::Arc,
  },
  thiserror::Error,
};

#[derive(Debug, Error)]
pub enum StateError {
  #[error("Writes are not supported on this type of state")]
  WritesNotSupported,

  #[error("Storage Engine Error: {0}")]
  StorageEngineError(#[from] crate::storage::Error),
}

type StateResult<T> = std::result::Result<T, StateError>;
type StorageResult<T> = std::result::Result<T, StorageError>;

/// Represents the state of the blockchain that is the result
/// of running the replicated state machine.
pub trait State {
  /// Retreives the account object and its contents from the state.
  fn get(&self, address: &Pubkey) -> Option<Account>;

  /// Stores or overwrites an account object and its contents in the state.
  fn set(
    &mut self,
    address: Pubkey,
    account: Account,
  ) -> StateResult<Option<Account>>;

  /// Stores or overwrites an account object and its contents in the state.
  fn remove(&mut self, address: Pubkey) -> StateResult<()>;

  /// Returns the CID or hash of the current state.
  ///
  /// Those CIDs are valid IPFS cids that can also be used
  /// for syncing blockchain state. Each State hash is a PB-DAG
  /// object that also points to the previous state that this
  /// state was built upon.
  fn hash(&self) -> Multihash;
}

pub trait StateStore: State {
  /// Applies a state diff from a finalized block
  fn apply(&self, diff: &StateDiff) -> StorageResult<()>;
}

/// Represents a view of two overlayed states without modifying any of them.
///
/// The entire state of the chain can be represented as a chain
/// of combined partial states produced by all transactions or blocks
/// executed in order.
pub struct Overlayed<'s1, 's2> {
  base: &'s1 dyn State,
  overlay: &'s2 dyn State,
}

impl<'s1, 's2> Overlayed<'s1, 's2> {
  /// Creates a new combines state view
  pub fn new(base: &'s1 dyn State, overlay: &'s2 dyn State) -> Self {
    Self { base, overlay }
  }
}

impl<'s1, 's2> State for Overlayed<'s1, 's2> {
  /// Retreives a value at a given key, first tries to get it from
  /// the overlay and then the base state.
  fn get(&self, address: &Pubkey) -> Option<Account> {
    match self.overlay.get(address) {
      None => self.base.get(address),
      Some(value) => Some(value),
    }
  }

  fn set(&mut self, _: Pubkey, _: Account) -> StateResult<Option<Account>> {
    Err(StateError::WritesNotSupported)
  }

  fn remove(&mut self, _: Pubkey) -> StateResult<()> {
    Err(StateError::WritesNotSupported)
  }

  fn hash(&self) -> Multihash {
    unimplemented!() // not applicable here
  }
}

/// Represents a block that has been finalized and is guaranteed
/// to never be reverted. It contains the global blockchain state.
#[derive(Debug)]
pub struct Finalized<'f, D: BlockData, S: StateStore> {
  underlying: Arc<dyn Block<D>>,
  state: &'f S,
}

impl<'f, D: BlockData, S: State + StateStore> Finalized<'f, D, S> {
  pub fn new(block: Arc<dyn Block<D>>, storage: &'f S) -> Self {
    Self {
      underlying: block,
      state: storage,
    }
  }

  pub fn apply(&mut self, block: Executed<D>) {
    assert!(block.parent == self.underlying.hash().unwrap());
    self.underlying = block.underlying;
    self
      .state
      .apply(&block.output.state)
      .expect("unrecoverable storage engine error"); // most likely disk is full
  }

  pub fn state(&self) -> &'f impl State {
    self.state
  }
}

impl<D: BlockData, S: StateStore> Deref for Finalized<'_, D, S> {
  type Target = Arc<dyn Block<D>>;

  fn deref(&self) -> &Self::Target {
    &self.underlying
  }
}

/// Represents a change in Blockchain Accounts state.
///
/// Statediff are meant to be accumulated and logically the entire
/// state of the blockchain is the result of cumulative application
/// of consecutive state diffs.
///
/// A transaction produces a statediff, blocks produce state diffs
/// which are all its transactions state diffs merged together.
/// If all blocks state diffs are also merged together, then the
/// resulting state diff would represent the entire state of the system.
///
/// StateDiff is also the basic unit of state sync through IPFS/bitswap.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StateDiff {
  data: BTreeMap<Pubkey, Account>,
  deletes: BTreeSet<Pubkey>,

  #[serde(skip)]
  hashcache: OnceCell<Multihash>,
}

impl StateDiff {
  pub fn merge(self, newer: StateDiff) -> StateDiff {
    let mut data = self.data;
    let mut deletes = self.deletes;
    for (addr, acc) in newer.data {
      deletes.remove(&addr);
      data.insert(addr, acc);
    }

    for addr in newer.deletes {
      deletes.insert(addr);
    }

    StateDiff {
      data,
      deletes,
      hashcache: OnceCell::new(),
    }
  }

  /// Iterate over all account changes in a state diff.
  ///
  /// There are two variants of changes:
  ///   1. (Address, Account) => Means that account under a given address was
  ///      created or changed its contents.
  ///   2. (Address, None) => Means that account under a given address was
  ///      deleted.
  pub fn iter(&self) -> impl Iterator<Item = (&Pubkey, Option<&Account>)> {
    self
      .data
      .iter()
      .map(|(addr, acc)| (addr, Some(acc)))
      .chain(self.deletes.iter().map(|addr| (addr, None)))
  }
}

impl State for StateDiff {
  fn get(&self, address: &Pubkey) -> Option<Account> {
    self.data.get(address).cloned()
  }

  fn set(
    &mut self,
    address: Pubkey,
    account: Account,
  ) -> StateResult<Option<Account>> {
    self.deletes.remove(&address);
    Ok(self.data.insert(address, account))
  }

  fn remove(&mut self, address: Pubkey) -> StateResult<()> {
    self.deletes.insert(address);
    Ok(())
  }

  fn hash(&self) -> Multihash {
    *self.hashcache.get_or_init(|| {
      let mut hasher = Sha3_256::default();
      for (k, v) in self.data.iter() {
        hasher.update(k.as_ref());
        hasher.update(&v.hash().to_bytes());
      }
      MultihashCode::Sha3_256.wrap(hasher.finalize()).unwrap()
    })
  }
}

#[cfg(test)]
mod test {
  use {
    super::{Overlayed, State, StateDiff},
    crate::primitives::{Account, Pubkey},
  };

  #[test]
  fn merge_test() {
    let mut s1 = StateDiff::default();
    let mut s2 = StateDiff::default();
    let mut s3 = StateDiff::default();

    let key1: Pubkey = "4AKRabNsRm6fgum4zmj5KH5qXGVLAKkxwr3U2Pt5ZXwF"
      .parse()
      .unwrap();

    let key2: Pubkey = "7jo1WoniBtewH7PuNhb8Lr58VuiGVEWPfaYiKffu9rYM"
      .parse()
      .unwrap();

    let key3 = "CrPhwoyRt3FobHtf4Hypss4R7tGLWaxgLbWdTMdpxZXv"
      .parse()
      .unwrap();

    assert!(s1.set(key1, Account::test_new(1)).is_ok());
    assert!(s2.set(key2, Account::test_new(2)).is_ok());
    assert!(s3.set(key1, Account::test_new(3)).is_ok());

    assert!(s1.get(&key3).is_none());
    assert!(s1.get(&key2).is_none());
    assert_eq!(s1.get(&key1), Some(Account::test_new(1)));

    assert!(s2.get(&key1).is_none());
    assert!(s2.get(&key3).is_none());
    assert_eq!(s2.get(&key2), Some(Account::test_new(2)));

    assert!(s3.get(&key2).is_none());
    assert!(s3.get(&key3).is_none());
    assert_eq!(s3.get(&key1), Some(Account::test_new(3)));

    let m12 = s1.merge(s2);
    assert!(m12.get(&key3).is_none());
    assert_eq!(m12.get(&key1), Some(Account::test_new(1)));
    assert_eq!(m12.get(&key2), Some(Account::test_new(2)));

    let m123 = m12.merge(s3);
    assert!(m123.get(&key3).is_none());
    assert_eq!(m123.get(&key2), Some(Account::test_new(2)));
    assert_eq!(m123.get(&key1), Some(Account::test_new(3))); // must override
  }

  #[test]
  fn combine_test() {
    let mut s1 = StateDiff::default();
    let mut s2 = StateDiff::default();
    let mut s3 = StateDiff::default();

    let key1: Pubkey = "4AKRabNsRm6fgum4zmj5KH5qXGVLAKkxwr3U2Pt5ZXwF"
      .parse()
      .unwrap();

    let key2: Pubkey = "7jo1WoniBtewH7PuNhb8Lr58VuiGVEWPfaYiKffu9rYM"
      .parse()
      .unwrap();

    let key3 = "CrPhwoyRt3FobHtf4Hypss4R7tGLWaxgLbWdTMdpxZXv"
      .parse()
      .unwrap();

    assert!(s1.set(key1, Account::test_new(1)).is_ok());
    assert!(s2.set(key2, Account::test_new(2)).is_ok());
    assert!(s3.set(key1, Account::test_new(3)).is_ok());

    assert!(s1.get(&key3).is_none());
    assert!(s1.get(&key2).is_none());
    assert_eq!(s1.get(&key1), Some(Account::test_new(1)));

    assert!(s2.get(&key1).is_none());
    assert!(s2.get(&key3).is_none());
    assert_eq!(s2.get(&key2), Some(Account::test_new(2)));

    assert!(s3.get(&key2).is_none());
    assert!(s3.get(&key3).is_none());
    assert_eq!(s3.get(&key1), Some(Account::test_new(3)));

    let c12 = Overlayed::new(&s1, &s2);
    assert!(c12.get(&key3).is_none());
    assert_eq!(c12.get(&key1), Some(Account::test_new(1)));
    assert_eq!(c12.get(&key2), Some(Account::test_new(2)));

    let c31 = Overlayed::new(&s3, &s1);
    assert!(c31.get(&key2).is_none());
    assert!(c31.get(&key3).is_none());
    assert_eq!(c31.get(&key1), Some(Account::test_new(1)));

    let c13 = Overlayed::new(&s1, &s3);
    assert!(c13.get(&key2).is_none());
    assert!(c13.get(&key3).is_none());
    assert_eq!(c13.get(&key1), Some(Account::test_new(3)));

    let c123 = Overlayed::new(&c12, &s3);
    assert!(c123.get(&key3).is_none());
    assert_eq!(c123.get(&key2), Some(Account::test_new(2)));
    assert_eq!(c123.get(&key1), Some(Account::test_new(3))); // newer wins

    let mut c12 = c12; // writes disabled on combined view
    assert!(c12.set(key3, Account::test_new(4)).is_err());
  }
}
