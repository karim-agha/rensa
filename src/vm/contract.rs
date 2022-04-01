//! Smart contracts programming VM interface
//!
//! This module defines the basic types that are used to
//! invoke smart contracts by the virtual machine and carry
//! input and output data into and from the contract.

use {
  super::{transaction::SignatureError, Machine},
  crate::primitives::Pubkey,
  serde::{Deserialize, Serialize},
  thiserror::Error,
};

#[derive(Debug, Error, Clone, Serialize, Deserialize)]
pub enum ContractError {
  #[error("Invalid transaction nonce value for this payer, expected {0}")]
  InvalidTransactionNonce(u64),

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

  #[error("This contract is not allowed to perform this operation")]
  UnauthorizedOperation,

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

#[derive(Debug, Clone)]
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

  /// Represents creation of a new account that is owned by a calling
  /// contract.
  ///
  /// The modified account should be set as writable in the transaction
  /// inputs, otherwise the transaction will fail.
  CreateOwnedAccount(Pubkey, Option<Vec<u8>>),

  /// Represents an overrwrite to the contents of an account owned
  /// by the contract.
  ///
  /// The modified account should be set as writable in the transaction
  /// inputs, otherwise the transaction will fail.
  ///
  /// To delete the data contents of an account without deleting the
  /// account itself (for example to reset it to some initial state),
  /// use [`None`] as the second parameter to this constructor.
  WriteAccountData(Pubkey, Option<Vec<u8>>),

  /// Represents a deletion of an account that is owned by the contract.
  ///
  /// The modified account should be set as writable in the transaction
  /// inputs, and its owner should match the executing contract.
  DeleteOwnedAccount(Pubkey),

  /// Represents a request for cross contract invocation to another contract.
  ContractInvoke {
    /// Address of the contract to be invoked
    contract: Pubkey,
    /// Input accounts to the contract to be invoked.
    ///
    /// Those accounts must already be referenced by the calling
    /// contract, with the same or higher writability flags.
    accounts: Vec<(Pubkey, AccountView)>,

    /// Input bytes to the invoked contract
    params: Vec<u8>,
  },

  /// Represents creation of a new contract account.
  ///
  /// This output is only allowed to be emitted by the WASM_VM contract
  /// during contract installation. Returning this value from any other
  /// contract will fail the entire transaction.
  CreateExecutableAccount(Pubkey, Vec<u8>),
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

/// This is the signature of a builtin contract entrypoint.
///
/// Builtin contracts have direct access to the virtual machine instance.
pub type NativeContractEntrypoint = fn(&Environment, &[u8], &Machine) -> Result;

/// This is the signature of a contract entrypoint.
///
/// WASM contracts run in an isolated environment and have no direct access
/// to any runtime facilities.
pub type ContractEntrypoint = fn(&Environment, &[u8]) -> Result;
