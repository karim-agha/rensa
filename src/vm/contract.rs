//! Smart contracts programming VM interface
//!
//! This module defines the basic types that are used to
//! invoke smart contracts by the virtual machine and carry
//! input and output data into and from the contract.

use thiserror::Error;

use crate::primitives::{Account, Pubkey};

#[derive(Debug, Error)]
pub enum ContractError {
  #[error("Account does not exist")]
  AccountDoesNotExist,

  #[error("The specified account is not writable")]
  _AccountNotWritable,

  #[error("The transaction has used up all compute units before completing")]
  _ComputationalBudgetExhausted,
}

/// This type represents a log entry emitted by a smart contract.
///
/// Log entries are key-value pairs that are emitted by a contract and
/// visible to external observers through the RPC interface.
#[derive(Debug)]
pub struct LogEntry(pub String, pub String);

/// Represents a modification to the contents of an account.
///
/// The modified account should be set as writable in the transaction
/// inputs, otherwise the transaction will fail.
#[derive(Debug)]
pub struct StateChange(pub Pubkey, pub Option<Account>);

// todo: add cross-contract invocation type.
#[derive(Debug)]
pub enum Output {
  LogEntry(LogEntry),
  StateChange(StateChange),
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
  pub address: Pubkey,
  pub accounts: Vec<(Pubkey, Option<Account>)>,
}

/// This is the signature of a contract entrypoint.
///
/// It is the same signature for builtin contracts and wasm contract
/// and any futute contract runtimes.
pub type ContractEntrypoint = fn(Environment, &[u8]) -> Result;
