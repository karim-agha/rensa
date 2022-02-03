use serde::{Deserialize, Serialize};

use super::Pubkey;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Account {
  pub balance: u64,
  #[serde(default)]
  pub executable: bool,
  pub owner: Option<Pubkey>,
  pub data: Option<Vec<u8>>,
}

impl Account {
  #[cfg(test)]
  pub fn test_new(value: u8) -> Self {
    Self {
      balance: 0,
      executable: false,
      owner: None,
      data: Some(vec![value]),
    }
  }
}
