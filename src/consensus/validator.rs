use crate::keys::Pubkey;

#[derive(Debug, Clone)]
pub struct Validator {
  pub pubkey: Pubkey,
  pub stake: u128,
}
