//! Smart contracts programming VM interface
//!
//! This module defines the basic types that are used to
//! invoke smart contracts by the virtual machine and carry
//! input and output data into and from the contract.

use {
  crate::primitives::{Account, Pubkey},
  thiserror::Error,
};

#[derive(Debug, Error)]
pub enum ContractError {
  #[error("Account already in use")]
  AccountAlreadyInUse,

  #[error("Invalid contract input paramters data")]
  InvalidInputParameters,

  #[error("Invalid contract input accounts")]
  InvalidInputAccounts,

  #[error("Account does not exist")]
  AccountDoesNotExist,

  #[error("Contract error: {0:?}")]
  Other(#[from] Box<dyn std::error::Error>),

  #[error("The specified account is not writable")]
  _AccountNotWritable,

  #[error("The transaction has used up all compute units before completing")]
  _ComputationalBudgetExhausted,
}

#[derive(Debug)]
pub enum Output {
  /// This type represents a log entry emitted by a smart contract.
  ///
  /// Log entries are key-value pairs that are emitted by a contract and
  /// visible to external observers through the RPC interface.
  LogEntry(String, String),

  /// Represents a modification to the contents of an account.
  ///
  /// The modified account should be set as writable in the transaction
  /// inputs, otherwise the transaction will fail.
  ModifyAccountData(Pubkey, Option<Vec<u8>>),

  /// Represents creation of a new account.
  ///
  /// The modified account should be set as writable in the transaction
  /// inputs, otherwise the transaction will fail.
  CreateAccount(Pubkey, Account),
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
  pub accounts: Vec<(Pubkey, Option<Account>)>,
}

/// This is the signature of a contract entrypoint.
///
/// It is the same signature for builtin contracts and wasm contract
/// and any futute contract runtimes.
pub type ContractEntrypoint = fn(Environment, &[u8]) -> Result;
