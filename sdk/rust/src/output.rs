use {
  crate::{env::AccountView, pubkey::Pubkey},
  borsh::BorshSerialize,
};

#[derive(Debug, BorshSerialize)]
pub enum Output {
  LogEntry(String, String),
  CreateOwnedAccount(Pubkey, Option<Vec<u8>>),
  WriteAccountData(Pubkey, Option<Vec<u8>>),
  DeleteOwnedAccount(Pubkey),
  ContractInvoke {
    contract: Pubkey,
    accounts: Vec<(Pubkey, AccountView)>,
    params: Vec<u8>,
  },
  CreateExecutableAccount(Pubkey, Vec<u8>),
}
