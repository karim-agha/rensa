use super::{Result, State};
use crate::primitives::{Account, Pubkey};
use multihash::Multihash;
use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct StateDiff {
  pub(super) data: HashMap<Pubkey, Account>,
}


impl StateDiff {
  pub fn merge(self, newer: StateDiff) -> StateDiff {
    StateDiff {
      data: self.data.into_iter().chain(newer.data).collect(),
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
    todo!()
  }
}

#[cfg(test)]
mod test {
  use super::StateDiff;
  use crate::{
    primitives::{Account, Pubkey},
    state::State,
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
}
