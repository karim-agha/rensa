pub mod builtin;
mod contract;
mod executed;
mod machine;
mod output;
mod runtime;
mod state;
mod transaction;
mod unit;

pub use {
  contract::ContractError,
  executed::Executed,
  machine::{Executable, Machine, MachineError},
  output::BlockOutput,
  state::{
    ApplicableState,
    Finalized,
    Overlayed,
    State,
    StateDiff,
    StateError,
  },
  transaction::{AccountRef, ExecutedTransaction, Transaction},
};

lazy_static::lazy_static! {
  /// Address of the only contract that is allowed to create executable accounts.
  pub static ref WASM_VM_BUILTIN_ADDR: crate::primitives::Pubkey =
    "WasmVM1xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
      .parse()
      .unwrap();
}
