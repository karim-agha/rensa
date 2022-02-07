//! VM builtin contracts
//!
//! Those are special contracts that are implemented by the VM in native code
//! and are exposed to the users of the chain. The invocation semantics are
//! identical to invoking a regular user-uploaded contract.

use {
  super::contract::{
    self,
    ContractEntrypoint,
    ContractError,
    Environment,
    LogEntry,
    Output,
    StateChange,
  },
  crate::primitives::Pubkey,
  multihash::{Hasher, Sha3_256},
  std::collections::HashMap,
};

lazy_static::lazy_static! {
  pub static ref BUILTIN_CONTRACTS: HashMap<Pubkey, ContractEntrypoint> = {
    let mut funcs = HashMap::<Pubkey, ContractEntrypoint>::new();
    funcs.insert("Sha3111111111111111111111111111111111111111".parse().unwrap(), sha3_test);
    funcs
  };
}

fn sha3_test(env: Environment, params: &[u8]) -> contract::Result {
  let mut sha = Sha3_256::default();
  if let Some(account) = &env.accounts[0].1 {
    if let Some(data) = &account.data {
      sha.update(data);
    } else {
      sha.update(params);
    }
    return Ok(vec![
      Output::StateChange(StateChange(
        env.accounts[0].0.clone(),
        Some(sha.finalize().to_vec()),
      )),
      Output::LogEntry(LogEntry("action".into(), "sha3".into())),
    ]);
  }

  Err(ContractError::AccountDoesNotExist)
}
