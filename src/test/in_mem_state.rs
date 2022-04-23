use {
  crate::{
    primitives::{Account, Pubkey},
    storage::Error as StorageError,
    vm::{State, StateDiff, StateError, StateStore},
  },
  multihash::Multihash,
  std::{self, cell::RefCell, collections::HashMap},
};

#[derive(Debug, Default)]
pub struct InMemState {
  pub(crate) db: RefCell<HashMap<Pubkey, Account>>,
}

impl State for InMemState {
  fn get(&self, address: &Pubkey) -> Option<Account> {
    self.db.borrow_mut().get(address).cloned()
  }

  fn set(
    &mut self,
    address: Pubkey,
    account: Account,
  ) -> Result<Option<Account>, StateError> {
    let account = self.db.borrow_mut().insert(address, account);
    Ok(account)
  }

  fn remove(&mut self, address: Pubkey) -> Result<(), StateError> {
    self.db.borrow_mut().remove(&address);
    Ok(())
  }

  fn hash(&self) -> Multihash {
    unimplemented!() // not applicable here, PersistenState also does not have
                     // an impl for this
  }
}

impl StateStore for InMemState {
  fn apply(&self, diff: StateDiff) -> std::result::Result<(), StorageError> {
    let mut db = self.db.borrow_mut();
    for (addr, account) in diff.into_iter() {
      match account {
        Some(account) => db.insert(addr, account),
        None => db.remove(&addr),
      };
    }

    Ok(())
  }
}
