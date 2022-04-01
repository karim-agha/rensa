use {
  crate::vm::{
    contract::{self, ContractError, Environment, Output},
    Machine,
  },
  multihash::{Hasher, Sha3_256},
};

pub fn contract(
  env: &Environment,
  params: &[u8],
  _: &Machine,
) -> contract::Result {
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
    Output::WriteAccountData(*addr, Some(sha.finalize().to_vec())),
    Output::LogEntry("action".into(), "sha3".into()),
  ])
}
