use borsh::BorshSerialize;

#[derive(Debug, BorshSerialize)]
pub enum SignatureError {
  InvalidSignature,
  MissingSigners,
}

#[derive(Debug, BorshSerialize)]
pub enum ContractError {
  InvalidTransactionNonce(u64),
  AccountAlreadyExists,
  AccountDoesNotExist,
  TooManyInputAccounts,
  AccountTooLarge,
  LogTooLarge,
  TooManyLogs,
  InvalidInputAccounts,
  InvalidAccountOwner,
  InvalidOutputAccount,
  AccountNotWritable,
  ContractDoesNotExit,
  AccountIsNotExecutable,
  SignatureError(SignatureError),
  InvalidInputParameters,
  UnauthorizedOperation,
  Runtime(String),
  Other(String),
  _ComputationalBudgetExhausted,
}
