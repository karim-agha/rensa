use {
  crate::{
    primitives::{Account, Pubkey},
    storage::Error as StorageError,
    vm::{State, StateDiff, StateError, StateStore},
  },
  multihash::Multihash,
  std::{self, collections::HashMap, sync::RwLock},
};

#[derive(Debug, Default)]
pub struct InMemState {
  pub db: RwLock<HashMap<Pubkey, Account>>,
}

impl State for InMemState {
  fn get(&self, address: &Pubkey) -> Option<Account> {
    self.db.write().unwrap().get(address).cloned()
  }

  fn set(
    &mut self,
    address: Pubkey,
    account: Account,
  ) -> Result<Option<Account>, StateError> {
    let account = self.db.write().unwrap().insert(address, account);
    Ok(account)
  }

  fn remove(&mut self, address: Pubkey) -> Result<(), StateError> {
    self.db.write().unwrap().remove(&address);
    Ok(())
  }

  fn hash(&self) -> Multihash {
    unimplemented!() // not applicable here, PersistenState also does not have
                     // an impl for this
  }
}

impl StateStore for InMemState {
  fn apply(&self, diff: &StateDiff) -> std::result::Result<(), StorageError> {
    let mut db = self.db.write().unwrap();
    for (addr, account) in diff.iter() {
      match account {
        Some(account) => db.insert(*addr, account.clone()),
        None => db.remove(addr),
      };
    }

    Ok(())
  }
}
