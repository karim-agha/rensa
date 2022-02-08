//! VM builtin contracts
//!
//! Those are special contracts that are implemented by the VM in native code
//! and are exposed to the users of the chain. The invocation semantics are
//! identical to invoking a regular user-uploaded contract.

mod currency;
mod sha3;

use {
  crate::{primitives::Pubkey, vm::contract::ContractEntrypoint},
  std::collections::HashMap,
};

lazy_static::lazy_static! {
  pub static ref BUILTIN_CONTRACTS: HashMap<Pubkey, ContractEntrypoint> = {
    let mut funcs = HashMap::<Pubkey, ContractEntrypoint>::new();
    funcs.insert("Sha3xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx".parse().unwrap(), sha3::contract);
    funcs.insert("Currency1xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx".parse().unwrap(), currency::contract);
    funcs
  };
}
