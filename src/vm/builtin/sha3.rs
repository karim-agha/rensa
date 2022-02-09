use {
  crate::vm::{
    contract,
    contract::{ContractError, Environment, Output},
  },
  multihash::{Hasher, Sha3_256},
};

pub fn contract(env: Environment, params: &[u8]) -> contract::Result {
  let mut sha = Sha3_256::default();

  if env.accounts.len() != 1 {
    return Err(ContractError::InvalidInputAccounts);
  }

  let (addr, accinfo) = &env.accounts[0];

  if !accinfo.writable {
    return Err(ContractError::AccountNotWritable);
  }

  // if has existing content hash it
  if let Some(ref data) = accinfo.data {
    sha.update(data);
  } else {
    // otherwise hash the initial value from params
    sha.update(params);
  }

  Ok(vec![
    Output::ModifyAccountData(addr.clone(), Some(sha.finalize().to_vec())),
    Output::LogEntry("action".into(), "sha3".into()),
  ])
}
