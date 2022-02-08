use {
  crate::vm::{
    contract,
    contract::{ContractError, Environment, LogEntry, Output, StateChange},
  },
  multihash::{Hasher, Sha3_256},
};

pub fn contract(env: Environment, params: &[u8]) -> contract::Result {
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
