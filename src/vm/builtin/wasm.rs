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
  Allocate {
    /// A seed value used to generate the contract address.
    /// The contract will be deployed at Wasm.derive(seed).
    seed: [u8; 32],

    /// The account that is allowed to upload and modify the contract
    /// bytecode.
    owner: Pubkey,

    /// Sha3 of the uncompressed WASM bytecode.
    checksum: [u8; 32],

    /// Size of the compressed WASM bytecode compressed using Zstd.
    ///
    /// The compression level of the code doesn't matter, as long
    /// as the checksum of the uncompressed bytecode matches the
    /// [`checksum`] value.
    ///
    /// The maximum size of data stored in this vec depends on the
    /// [`max_contract_size`] value in Genesis.
    size: u32,
  },
}

/// This builtin contract allows external users of the blockchain to upload
/// and deploy new contracts WASM bytecode. Blockchain bytecode is gzipped
/// before upload and decompressed on the validator after receipt of the
/// deployment transaction.
pub fn contract(_env: &Environment, _params: &[u8]) -> contract::Result {
  todo!();
}
