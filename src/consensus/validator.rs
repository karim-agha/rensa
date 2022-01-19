use crate::keys::Pubkey;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Validator {
  pub pubkey: Pubkey,
  pub stake: u128,
}
