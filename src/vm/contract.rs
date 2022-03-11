//! Smart contracts programming VM interface
//!
//! This module defines the basic types that are used to
//! invoke smart contracts by the virtual machine and carry
//! input and output data into and from the contract.

use {
  super::transaction::SignatureError,
  crate::primitives::Pubkey,
  serde::{Deserialize, Serialize},
  thiserror::Error,
};

#[derive(Debug, Error, Serialize, Deserialize)]
pub enum ContractError {
  #[error("Account already exists")]
  AccountAlreadyExists,

  #[error("Account does not exist")]
  AccountDoesNotExist,

  #[error("The number of input accounts exceeds the maximum limit")]
  TooManyInputAccounts,

  #[error("Account is exceeding the maximum size limit")]
  AccountTooLarge,

  #[error("Log is exceeding the maximum size limit")]
  LogTooLarge,

  #[error("Logs count for this transaction is exceeding the maximum limit")]
  TooManyLogs,

  #[error("Invalid contract input accounts")]
  InvalidInputAccounts,

  #[error("Attempt to modify an account not owned by the contract")]
  InvalidAccountOwner,

  #[error(
    "Contract attempting to write to an account not in the transaction \
     accounts list"
  )]
  InvalidOutputAccount,

  #[error("The specified account is not writable")]
  AccountNotWritable,

  #[error("Contract does not exit")]
  ContractDoesNotExit,

  #[error("Signature Error: {0}")]
  SignatureError(#[from] SignatureError),

  #[error("Invalid contract input paramters data")]
  InvalidInputParameters,

  #[error("Contract error: {0}")]
  Other(String),

  #[error("The transaction has used up all compute units before completing")]
  _ComputationalBudgetExhausted,
}

impl From<std::io::Error> for ContractError {
  fn from(e: std::io::Error) -> Self {
    ContractError::Other(e.to_string())
  }
}

#[derive(Debug)]
pub struct AccountView {
  pub signer: bool,
  pub writable: bool,
  pub executable: bool,
  pub owner: Option<Pubkey>,
  pub data: Option<Vec<u8>>,
}

#[derive(Debug)]
pub enum Output {
  /// This type represents a log entry emitted by a smart contract.
  ///
  /// Log entries are key-value pairs that are emitted by a contract and
  /// visible to external observers through the RPC interface.
  LogEntry(String, String),

  /// Represents a modification to the contents of an account owned
  /// by the contract.
  ///
  /// The modified account should be set as writable in the transaction
  /// inputs, otherwise the transaction will fail.
  ModifyAccountData(Pubkey, Option<Vec<u8>>),

  /// Represents creation of a new account that is owned by a calling
  /// contract.
  ///
  /// The modified account should be set as writable in the transaction
  /// inputs, otherwise the transaction will fail.
  CreateOwnedAccount(Pubkey, Option<Vec<u8>>),
}

/// Represents the output of invocing a smart contract by a transaction.
/// The output is a list of either log entries or state changes on success, or
/// an error code on failure.
pub type Result = std::result::Result<Vec<Output>, ContractError>;

/// This is the self-cointained input type that is passed to the
/// contract code containing all accounts data referenced by the
/// transaction.
#[derive(Debug)]
pub struct Environment {
  /// Address of the contract that is being invoked
  pub address: Pubkey,

  /// A list of all input accounts specified by the transaction
  pub accounts: Vec<(Pubkey, AccountView)>,
}

/// This is the signature of a contract entrypoint.
///
/// It is the same signature for builtin contracts and wasm contract
/// and any futute contract runtimes.
pub type ContractEntrypoint = fn(&Environment, &[u8]) -> Result;
