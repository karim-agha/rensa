use {
  super::Executed,
  crate::{
    consensus::{Block, BlockData, Genesis},
    primitives::{Account, Pubkey},
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
    collections::{BTreeMap, HashMap},
    ops::Deref,
  },
  thiserror::Error,
};

#[derive(Debug, Error)]
pub enum StateError {
  #[error("Unknown state error")]
  Unknown,

  #[error("Writes are not supported on this type of state")]
  WritesNotSupported,

  #[error("Cannot apply state: parent hash mismatch")]
  ParentMismatch,
}

type Result<T> = std::result::Result<T, StateError>;

/// Represents the state of the blockchain that is the result
/// of running the replicated state machine.
pub trait State {
  /// Retreives the account object and its contents from the state.
  fn get(&self, address: &Pubkey) -> Option<&Account>;

  /// Stores or overwrites an account object and its contents in the state.
  fn set(
    &mut self,
    address: Pubkey,
    account: Account,
  ) -> Result<Option<Account>>;

  /// Returns the CID or hash of the current state.
  ///
  /// Those CIDs are valid IPFS cids that can also be used
  /// for syncing blockchain state. Each State hash is a PB-DAG
  /// object that also points to the previous state that this
  /// state was built upon.
  fn hash(&self) -> Multihash;
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
  fn get(&self, address: &Pubkey) -> Option<&Account> {
    match self.overlay.get(address) {
      None => self.base.get(address),
      Some(value) => Some(value),
    }
  }

  fn set(&mut self, _: Pubkey, _: Account) -> Result<Option<Account>> {
    Err(StateError::WritesNotSupported)
  }

  fn hash(&self) -> Multihash {
    unimplemented!() // not applicable here
  }
}

/// Represents state of the blockchain at the last finalized
/// block. This state is persisted to disk and is not affected
/// by blockchain forks in the consensus.
///
/// Data in this state is large (counted in GBs). It gets updated
/// by applying StateDiffs to it from newly finalized blocks.
#[derive(Debug, Serialize, Deserialize)]
pub struct FinalizedState {
  data: BTreeMap<Pubkey, Account>,
}

impl FinalizedState {
  pub fn new(data: BTreeMap<Pubkey, Account>) -> Self {
    Self { data }
  }

  pub fn apply(&mut self, diff: StateDiff) {
    for (k, v) in diff.data {
      self.data.insert(k, v);
    }
  }
}

impl State for FinalizedState {
  fn get(&self, address: &Pubkey) -> Option<&Account> {
    self.data.get(address)
  }

  /// Writes directly to finalized state are not supported, instead
  /// state diffs from newly finalized blocks should be applied using the
  /// [`apply`] method
  fn set(&mut self, _: Pubkey, _: Account) -> Result<Option<Account>> {
    Err(StateError::WritesNotSupported)
  }

  /// The data hash of the entire finalized state.
  ///
  /// This field is simlar in its purpose to a merkle tree in ethereum,
  /// except it also represents valid IPFS CIDv1 PB-DAG entries, that can
  /// be used to sync blockchain state up to this point from other peers
  /// or from external IPFS pinning services.
  fn hash(&self) -> Multihash {
    todo!()
  }
}

/// Represents a block that has been finalized and is guaranteed
/// to never be reverted. It contains the global blockchain state.
#[derive(Debug)]
pub struct Finalized<D: BlockData> {
  underlying: Box<dyn Block<D>>,
  state: FinalizedState,
}

impl<D: BlockData> Finalized<D> {
  pub fn new(genesis: &Genesis<D>) -> Self {
    Self {
      underlying: Box::new(genesis.clone()),
      state: FinalizedState::new(genesis.state.clone()),
    }
  }

  pub fn apply(&mut self, block: Executed<D>) {
    assert!(block.parent == self.underlying.hash().unwrap());
    self.underlying = Box::new(block.underlying);
    self.state.apply(block.state_diff);
  }

  pub fn state(&self) -> &impl State {
    &self.state
  }
}

impl<D: BlockData> Deref for Finalized<D> {
  type Target = Box<dyn Block<D>>;

  fn deref(&self) -> &Self::Target {
    &self.underlying
  }
}

#[derive(Debug, Clone, Default)]
pub struct StateDiff {
  data: BTreeMap<Pubkey, Account>,
  hashcache: OnceCell<Multihash>,
}

impl StateDiff {
  pub fn merge(self, newer: StateDiff) -> StateDiff {
    StateDiff {
      data: self.data.into_iter().chain(newer.data).collect(),
      hashcache: OnceCell::new(),
    }
  }
}

impl State for StateDiff {
  fn get(&self, address: &Pubkey) -> Option<&Account> {
    self.data.get(address)
  }

  fn set(
    &mut self,
    address: Pubkey,
    account: Account,
  ) -> Result<Option<Account>> {
    Ok(self.data.insert(address, account))
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

pub struct IsolatedState {
  data: HashMap<Pubkey, Account>,
}

impl IsolatedState {
  pub fn new(_base: &impl State, _accounts: &[Pubkey]) -> Result<Self> {
    todo!();
  }
}

impl State for IsolatedState {
  fn get(&self, address: &Pubkey) -> Option<&Account> {
    self.data.get(address)
  }

  fn set(
    &mut self,
    address: Pubkey,
    account: Account,
  ) -> Result<Option<Account>> {
    Ok(self.data.insert(address, account))
  }

  fn hash(&self) -> Multihash {
    todo!()
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

    assert!(s1.set(key1.clone(), Account::test_new(1)).is_ok());
    assert!(s2.set(key2.clone(), Account::test_new(2)).is_ok());
    assert!(s3.set(key1.clone(), Account::test_new(3)).is_ok());

    assert!(s1.get(&key3).is_none());
    assert!(s1.get(&key2).is_none());
    assert_eq!(s1.get(&key1), Some(&Account::test_new(1)));

    assert!(s2.get(&key1).is_none());
    assert!(s2.get(&key3).is_none());
    assert_eq!(s2.get(&key2), Some(&Account::test_new(2)));

    assert!(s3.get(&key2).is_none());
    assert!(s3.get(&key3).is_none());
    assert_eq!(s3.get(&key1), Some(&Account::test_new(3)));

    let m12 = s1.merge(s2);
    assert!(m12.get(&key3).is_none());
    assert_eq!(m12.get(&key1), Some(&Account::test_new(1)));
    assert_eq!(m12.get(&key2), Some(&Account::test_new(2)));

    let m123 = m12.merge(s3);
    assert!(m123.get(&key3).is_none());
    assert_eq!(m123.get(&key2), Some(&Account::test_new(2)));
    assert_eq!(m123.get(&key1), Some(&Account::test_new(3))); // must override
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

    assert!(s1.set(key1.clone(), Account::test_new(1)).is_ok());
    assert!(s2.set(key2.clone(), Account::test_new(2)).is_ok());
    assert!(s3.set(key1.clone(), Account::test_new(3)).is_ok());

    assert!(s1.get(&key3).is_none());
    assert!(s1.get(&key2).is_none());
    assert_eq!(s1.get(&key1), Some(&Account::test_new(1)));

    assert!(s2.get(&key1).is_none());
    assert!(s2.get(&key3).is_none());
    assert_eq!(s2.get(&key2), Some(&Account::test_new(2)));

    assert!(s3.get(&key2).is_none());
    assert!(s3.get(&key3).is_none());
    assert_eq!(s3.get(&key1), Some(&Account::test_new(3)));

    let c12 = Overlayed::new(&s1, &s2);
    assert!(c12.get(&key3).is_none());
    assert_eq!(c12.get(&key1), Some(&Account::test_new(1)));
    assert_eq!(c12.get(&key2), Some(&Account::test_new(2)));

    let c31 = Overlayed::new(&s3, &s1);
    assert!(c31.get(&key2).is_none());
    assert!(c31.get(&key3).is_none());
    assert_eq!(c31.get(&key1), Some(&Account::test_new(1)));

    let c13 = Overlayed::new(&s1, &s3);
    assert!(c13.get(&key2).is_none());
    assert!(c13.get(&key3).is_none());
    assert_eq!(c13.get(&key1), Some(&Account::test_new(3)));

    let c123 = Overlayed::new(&c12, &s3);
    assert!(c123.get(&key3).is_none());
    assert_eq!(c123.get(&key2), Some(&Account::test_new(2)));
    assert_eq!(c123.get(&key1), Some(&Account::test_new(3))); // newer wins

    let mut c12 = c12; // writes disabled on combined view
    assert!(c12.set(key3, Account::test_new(4)).is_err());
  }
}
