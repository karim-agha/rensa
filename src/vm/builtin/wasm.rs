//! Wasm VM Contract
//!
//! This builtin contract implements WASM smart contract deployment and update.

use {
  crate::{
    primitives::Pubkey,
    vm::contract::{self, Environment},
  },
  borsh::{BorshDeserialize, BorshSerialize},
  serde::Deserialize,
};

/// This is the instruction param to the wasm deployment contract
#[derive(Debug, Deserialize, BorshSerialize, BorshDeserialize)]
enum Instruction {
  /// Deploys a new WASM smart contract
  ///
  /// Accounts expected by this instruction:
  ///   0. [drw-] Contract destination address [Wasm.derive(seed)]
  ///   1. [drw-] Contract bytecode storage address
  ///       [Wasm.derive(seed, b"bytecode")]
  Allocate {
    /// A seed value used to generate the contract address.
    /// The contract will be deployed at Wasm.derive(seed).
    seed: [u8; 32],

    /// Sha3 of the final uncompressed WASM bytecode.
    checksum: [u8; 32],

    /// The account that is allowed to upload pieces of the contract code
    authority: Pubkey,

    /// Uncompressed WASM bytecode size.
    ///
    /// The compression level of the code doesn't matter, as long
    /// as the checksum of the uncompressed bytecode matches the
    /// [`checksum`] value.
    ///
    /// The maximum size of data stored in this vec depends on the
    /// [`max_contract_size`] value in Genesis.
    size: u32,
  },

  /// Uploads a piece of the wasm bytecode and stores it in the bytecode store.
  /// Each chunk is 2kb in size max.
  ///
  /// Accounts expected by this instruction:
  ///   0. [drw-] Contract destination address [Wasm.derive(seed)]
  ///   1. [drw-] Contract bytecode storage address
  ///       [Wasm.derive(seed, b"bytecode")]   
  ///   2. [---s] Signature of the authority account specified during
  ///       [`Allocate`]
  ///
  /// This transaction will fail if it tries to overwrite an already uploaded
  /// slot.
  Upload {
    /// A seed value used to generate the contract address.
    /// The contract will be deployed at Wasm.derive(seed).
    seed: [u8; 32],

    /// Index of the stored bytes.
    ///
    /// Each slot is 2kb except the last slot could be smaller,
    /// so the storage position is [2kb * index, 2kb*index + len]
    index: u16,

    /// A piece of the bytecode at a given position with a maximum length of
    /// 2kb.
    bytes: Vec<u8>,
  },

  /// Once all the contract bytecode is uploaded, this instruction
  /// creates an executable contract out of it and optionally invokes
  /// the contract.
  ///
  /// Accounts expected by this instruction:
  ///   0. [drw-] Contract destination address [Wasm.derive(seed)]
  ///   1. [drw-] Contract bytecode storage address
  ///       [Wasm.derive(seed, b"bytecode")]   
  ///   2. [---s] Signature of the authority account specified during
  ///       [`Allocate`]
  ///
  /// This instruction will fail if:
  ///   - not all parts of the bytecode were uploaded.
  ///   - The uploaded bytecode is not a valid WASM.
  ///
  /// Once the bytecode is installed as an executable, the bytecode
  /// buffer gets deleted and moved to the contract address.
  Install {
    /// A seed value used to generate the contract address.
    /// The contract will be deployed at Wasm.derive(seed).
    seed: [u8; 32],

    /// An optional instruction that is invoked on the contract once
    /// it is deployed as an executable.
    init: Option<Vec<u8>>,
  },
}

/// This builtin contract allows external users of the blockchain to upload
/// and deploy new contracts WASM bytecode.
pub fn contract(_env: &Environment, _params: &[u8]) -> contract::Result {
  todo!();
}
