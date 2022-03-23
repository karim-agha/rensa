use crate::vm::contract::{self, Environment};

/// This builtin contract allows external users of the blockchain to upload 
/// and deploy new contracts bytecode. Blockchain bytecode is split into small
/// batches of blobs and uploaded as a series of transactions until the entire
/// code is on chain, and then the contract becomes callable when all pieces
/// are stored and confirmed by the chain.
pub fn contract(_env: &Environment, _params: &[u8]) -> contract::Result {
  todo!();
}
