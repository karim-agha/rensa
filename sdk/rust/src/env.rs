use {
  crate::pubkey::Pubkey,
  borsh::{BorshDeserialize, BorshSerialize},
};

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct AccountView {
  pub signer: bool,
  pub writable: bool,
  pub executable: bool,
  pub owner: Option<Pubkey>,
  pub data: Option<Vec<u8>>,
}

#[derive(Debug, BorshDeserialize)]
pub struct Environment {
  pub caller: Option<Pubkey>,
  pub address: Pubkey,
  pub accounts: Vec<(Pubkey, AccountView)>,
}
