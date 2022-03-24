use crate::vm::contract::{self, Environment};

/// This builtin contract allows external users of the blockchain to upload 
/// and deploy new contracts WASM bytecode. Blockchain bytecode is gzipped 
/// before upload and decompressed on the validator after receipt of the 
/// deployment transaction.
pub fn contract(_env: &Environment, _params: &[u8]) -> contract::Result {
  todo!();
}
