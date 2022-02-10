use {
  super::{
    builtin::BUILTIN_CONTRACTS,
    contract::ContractEntrypoint,
    unit::ExecutionUnit,
    Overlayed,
    State,
    StateDiff,
    Transaction,
  },
  crate::{
    consensus::{BlockData, Genesis, Produced},
    primitives::{Pubkey, ToBase58String},
  },
  std::collections::HashMap,
  thiserror::Error,
  tracing::warn,
};

#[derive(Debug, Error)]
pub enum MachineError {
  #[error(
    "The resulting state diff of this block is inconsistent with the state \
     hash decalred in the block header"
  )]
  InconsistentStateHash,

  #[error("Invalid block height, expected a monotonically increasing value")]
  InvalidBlockHeight,

  #[error("Undefined builtin in genesis: {0}")]
  UndefinedBuiltin(Pubkey),
}

pub trait Executable {
  fn execute(
    &self,
    vm: &Machine,
    state: &dyn State,
  ) -> Result<StateDiff, MachineError>;
}

/// Represents a state machine that takes as an input a state
/// and a block and outputs a new state. This is the API
/// entry point to the virtual machine that runs contracts.
pub struct Machine {
  max_input_accounts: usize,
  builtins: HashMap<Pubkey, ContractEntrypoint>,
}

impl Machine {
  pub fn new<D: BlockData>(genesis: &Genesis<D>) -> Result<Self, MachineError> {
    let max_input_accounts = genesis.max_input_accounts;
    let mut builtins = HashMap::new();
    for addr in &genesis.builtins {
      if let Some(entrypoint) = BUILTIN_CONTRACTS.get(addr) {
        builtins.insert(*addr, *entrypoint);
      } else {
        return Err(MachineError::UndefinedBuiltin(*addr));
      }
    }
    Ok(Self {
      builtins,
      max_input_accounts,
    })
  }

  /// Gets a VM-native builtin contract.
  /// Those contracts have to be enabled in the genesis config.
  pub fn builtin(&self, addr: &Pubkey) -> Option<&ContractEntrypoint> {
    self.builtins.get(addr)
  }

  /// The maximum number of accounts a transaction can reference.
  pub fn max_input_accounts(&self) -> usize {
    self.max_input_accounts
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
    state: &dyn State,
  ) -> Result<StateDiff, MachineError> {
    let mut accstate = StateDiff::default();
    for transaction in self {
      // Create a view of the state that encompasses the global state
      // and the state accumulated so far by the block.
      let state = Overlayed::new(state, &accstate);

      // try instantiating the contract, construct its
      // isolated environment and execute it then injest
      // all its outputs if ran successfully to completion.
      match ExecutionUnit::new(transaction, &state, vm)
        .and_then(|exec_unit| exec_unit.execute())
      {
        Ok(statediff) => {
          // transaction execution successfully ran to completion.
          // merge and accumulate state changes in this block.
          accstate = accstate.merge(statediff);
        }
        Err(error) => {
          // on error, don't apply any of transaction changes
          warn!(
            "transaction {} failed: {error}",
            transaction.hash().to_b58()
          );
        }
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
    _state: &dyn State,
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
    _state: &dyn State,
  ) -> Result<StateDiff, MachineError> {
    Ok(StateDiff::default())
  }
}
