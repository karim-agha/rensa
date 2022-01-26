use super::{Result, State};
use crate::primitives::{Account, Pubkey};
use std::collections::HashMap;

use multihash::Multihash;

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
