use std::cmp::Ordering;

use crate::primitives::Pubkey;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Validator {
  pub pubkey: Pubkey,
  pub stake: u64,
}

impl Eq for Validator {}
impl PartialEq for Validator {
  fn eq(&self, other: &Self) -> bool {
    self.pubkey == other.pubkey && self.stake == other.stake
  }
}

impl PartialOrd for Validator {
  fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
    match self.pubkey.partial_cmp(&other.pubkey) {
      Some(core::cmp::Ordering::Equal) => self.stake.partial_cmp(&other.stake),
      ord => ord,
    }
  }
}

impl Ord for Validator {
  fn cmp(&self, other: &Self) -> std::cmp::Ordering {
    match self.pubkey.cmp(&other.pubkey) {
      Ordering::Equal => self.stake.cmp(&other.stake),
      ord => ord,
    }
  }
}
