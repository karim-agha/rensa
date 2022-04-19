use {
  borsh::BorshDeserialize,
  rensa_sdk::{log, main, ContractError, Environment, Output, Pubkey},
};

#[derive(Debug, BorshDeserialize)]
pub enum Instruction {
  Register { name: String, owner: Pubkey },
  Update { name: String, owner: Pubkey },
  Release { name: String },
}

#[main]
fn main(
  env: &Environment,
  params: &[u8],
) -> Result<Vec<Output>, ContractError> {
  log(&format!("environment object: {env:?}"));

  if params.is_empty() {
    return Err(ContractError::InvalidInputParameters);
  }

  let instruction = Instruction::try_from_slice(params).unwrap();
  log(&format!("instruction: {instruction:?}"));

  if let Instruction::Release { name } = instruction {
    return Err(ContractError::Other(format!(
      "Release is not implemented for {name}"
    )));
  }

  Ok(vec![
    Output::LogEntry("test-key".into(), "test-value".into()),
    Output::CreateOwnedAccount(env.address, Some(vec![1, 2, 3])),
  ])
}
