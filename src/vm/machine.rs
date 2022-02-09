use {
  super::{
    builtin::BUILTIN_CONTRACTS,
    contract::{ContractEntrypoint, Environment, Output},
    Overlayed,
    State,
    StateDiff,
    Transaction,
  },
  crate::{
    consensus::{BlockData, Genesis, Produced},
    primitives::{Account, Pubkey, ToBase58String},
  },
  std::collections::HashMap,
  thiserror::Error,
  tracing::debug,
};

#[derive(Debug, Error)]
pub enum MachineError {
  #[error("Unknown error")]
  UnknownError,

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
    state: &dyn State,
  ) -> Result<StateDiff, MachineError> {
    let mut accstate = StateDiff::default();
    for transaction in self {
      if let Some(entrypoint) = vm.builtins.get(&transaction.contract) {
        let state = Overlayed::new(state, &accstate);
        let env = Environment {
          address: transaction.contract.clone(),
          accounts: transaction
            .accounts
            .iter()
            .map(|a| (a.address.clone(), state.get(&a.address).cloned()))
            .collect(),
        };
        match entrypoint(env, &transaction.params) {
          Ok(outputs) => {
            let mut txstate = StateDiff::default();
            for output in outputs {
              match output {
                Output::LogEntry(key, value) => {
                  debug!(
                    "transaction {} log: {key} => {value}",
                    transaction.hash().to_b58()
                  ); // todo
                }
                Output::ModifyAccountData(addr, data) => {
                  for acc in &transaction.accounts {
                    if acc.address == addr && acc.writable {
                      if let Some(acc) = state.get(&addr) {
                        txstate
                          .set(addr.clone(), Account {
                            data,
                            ..acc.clone()
                          })
                          .unwrap();
                        break;
                      }
                    }
                  }
                }
                Output::CreateAccount(_addr, _acc) => {}
              };
            }
            accstate = accstate.merge(txstate);
          }
          Err(error) => {
            // when a transaction fails, none of its state changes
            // gets persisted. Todo: Add failure logs.
            debug!(
              "transaction {} failed: {error:?}",
              transaction.hash().to_b58()
            );
          }
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
