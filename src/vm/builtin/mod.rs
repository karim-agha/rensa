//! VM builtin contracts
//!
//! Those are special contracts that are implemented by the VM in native code
//! and are exposed to the users of the chain. The invocation semantics are
//! identical to invoking a regular user-uploaded contract.

pub mod currency;
mod sha3;
mod staking;
mod wasm;

use {
  crate::{primitives::Pubkey, vm::contract::NativeContractEntrypoint},
  std::collections::HashMap,
};

lazy_static::lazy_static! {
  pub static ref BUILTIN_CONTRACTS: HashMap<Pubkey, NativeContractEntrypoint> = {
    let mut funcs = HashMap::<Pubkey, NativeContractEntrypoint>::new();
    funcs.insert("Sha3xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx".parse().unwrap(), sha3::contract);
    funcs.insert("Currency1xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx".parse().unwrap(), currency::contract);
    funcs.insert("Staking1xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx".parse().unwrap(), staking::contract);
    funcs.insert("WasmVM1xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx".parse().unwrap(), wasm::contract);
    funcs
  };
}
