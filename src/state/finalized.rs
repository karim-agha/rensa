use multihash::Multihash;

use super::{Result, State};
use crate::primitives::{Account, Pubkey};

pub struct FinalizedState;

impl State for FinalizedState {
  fn get(&self, _address: &Pubkey) -> Option<&Account> {
    todo!()
  }

  fn set(
    &mut self,
    _address: Pubkey,
    _account: Account,
  ) -> Result<Option<Account>> {
    todo!()
  }

  fn hash(&self) -> Multihash {
    todo!()
  }
}
