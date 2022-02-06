use {
  super::{
    builtin::BUILTIN_CONTRACTS,
    contract::ContractEntrypoint,
    State,
    StateDiff,
    Transaction,
  },
  crate::{
    consensus::{BlockData, Genesis, Produced},
    primitives::Pubkey,
  },
  std::collections::HashMap,
  thiserror::Error,
  tracing::info,
};

#[derive(Debug, Error)]
pub enum MachineError {
  #[error("Unknown error")]
  UnknownError,

  #[error("Undefined builtin in genesis: {0}")]
  UndefinedBuiltin(Pubkey),
}

pub trait Executable {
  fn execute(
    &self,
    vm: &Machine,
    state: &impl State,
  ) -> Result<StateDiff, MachineError>;
}

/// Represents a state machine that takes as an input a state
/// and a block and outputs a new state. This is the API
/// entry point to the virtual machine that runs contracts.
pub struct Machine {
  builtins: HashMap<Pubkey, ContractEntrypoint>,
}

impl Machine {
  pub fn new<D: BlockData>(genesis: &Genesis<D>) -> Result<Self, MachineError> {
    let mut builtins = HashMap::new();
    for addr in &genesis.builtins {
      if let Some(entrypoint) = BUILTIN_CONTRACTS.get(addr) {
        builtins.insert(addr.clone(), *entrypoint);
      } else {
        return Err(MachineError::UndefinedBuiltin(addr.clone()));
      }
    }
    Ok(Self { builtins })
  }

  pub fn execute<D: BlockData>(
    &self,
    state: &impl State,
    block: &Produced<D>,
  ) -> Result<StateDiff, MachineError> {
    block.data.execute(self, state)
  }
}

/// An implementation for blocks that carry a list of transactions.
impl Executable for Vec<Transaction> {
  fn execute(
    &self,
    vm: &Machine,
    _state: &impl State,
  ) -> Result<StateDiff, MachineError> {
    let accstate = StateDiff::default();
    for transaction in self {
      if let Some(_contract) = vm.builtins.get(&transaction.contract) {
        // todo
      }
    }
    Ok(accstate)
  }
}

// used in unit tests only
#[cfg(test)]
impl Executable for String {
  fn execute(
    &self,
    _vm: &Machine,
    _state: &impl State,
  ) -> Result<StateDiff, MachineError> {
    Ok(StateDiff::default())
  }
}

// used in unit tests only
#[cfg(test)]
impl Executable for u8 {
  fn execute(
    &self,
    _vm: &Machine,
    _state: &impl State,
  ) -> Result<StateDiff, MachineError> {
    Ok(StateDiff::default())
  }
}
