use {
  super::{
    builtin::BUILTIN_CONTRACTS,
    contract::ContractEntrypoint,
    output::{BlockOutput, ErrorsMap, LogsMap},
    unit::ExecutionUnit,
    Overlayed,
    State,
    StateDiff,
    Transaction,
  },
  crate::{
    consensus::{BlockData, Genesis, Limits, Produced},
    primitives::{Account, Pubkey, ToBase58String},
  },
  std::collections::HashMap,
  thiserror::Error,
  tracing::debug,
};

#[derive(Debug, Error)]
pub enum MachineError {
  #[error(
    "The resulting state diff of this block is inconsistent with the state \
     hash decalred in the block header"
  )]
  InconsistentStateHash,

  #[error(
    "The resulting logs of this block are inconsistent with the state hash \
     decalred in the block header"
  )]
  InconsistentLogsHash,

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
  ) -> Result<BlockOutput, MachineError>;
}

/// Represents a state machine that takes as an input a state
/// and a block and outputs a new state. This is the API
/// entry point to the virtual machine that runs contracts.
pub struct Machine {
  limits: Limits,
  builtins: HashMap<Pubkey, ContractEntrypoint>,
}

impl Machine {
  pub fn new<D: BlockData>(genesis: &Genesis<D>) -> Result<Self, MachineError> {
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
      limits: genesis.limits.clone(),
    })
  }

  /// Gets a VM-native builtin contract.
  /// Those contracts have to be enabled in the genesis config.
  pub fn builtin(&self, addr: &Pubkey) -> Option<&ContractEntrypoint> {
    self.builtins.get(addr)
  }

  /// Configured execution contraints.
  pub fn limits(&self) -> &Limits {
    &self.limits
  }

  pub fn execute<D: BlockData>(
    &self,
    state: &impl State,
    block: &Produced<D>,
  ) -> Result<BlockOutput, MachineError> {
    block.data.execute(self, state)
  }
}

/// An implementation for blocks that carry a list of transactions.
impl Executable for Vec<Transaction> {
  fn execute(
    &self,
    vm: &Machine,
    state: &dyn State,
  ) -> Result<BlockOutput, MachineError> {
    // accumulates state across all txs
    let mut acclogs = LogsMap::new();
    let mut accerrors = ErrorsMap::new();
    let mut accstate = StateDiff::default();

    for transaction in self {
      // on execution of a tranasction, increment payer's nonce value
      // so the same transaction could not be replayed in the future,
      // regardless of its execution outcome.
      match Overlayed::new(state, &accstate).get(&transaction.payer) {
        Some(mut payer) => {
          payer.nonce += 1;
          accstate.set(transaction.payer, payer).unwrap();
        }
        None => {
          accstate
            .set(transaction.payer, Account {
              nonce: 1,
              ..Account::default()
            })
            .unwrap();
        }
      };

      // Create a view of the state that encompasses the global state
      // and the state accumulated so far by the block.
      let state = Overlayed::new(state, &accstate);

      // try instantiating the contract, construct its
      // isolated environment and execute it then injest
      // all its outputs if ran successfully to completion.

      match ExecutionUnit::new(transaction, &state, vm)
        .and_then(|exec_unit| exec_unit.execute())
      {
        Ok(txout) => {
          // transaction execution successfully ran to completion.
          // merge and accumulate state changes in this block.
          accstate = accstate.merge(txout.state_diff);

          // append all generated logs
          acclogs.insert(*transaction.hash(), txout.log_entries);
        }
        Err(error) => {
          // on error, don't apply any of transaction changes
          debug!(
            "transaction {} failed: {error}",
            transaction.hash().to_b58()
          );

          // store the error output of the failed transaction
          accerrors.insert(*transaction.hash(), error);
        }
      };
    }

    Ok(BlockOutput::new(accstate, acclogs, accerrors))
  }
}

// used in unit tests only
#[cfg(test)]
impl Executable for String {
  fn execute(
    &self,
    _vm: &Machine,
    _state: &dyn State,
  ) -> Result<BlockOutput, MachineError> {
    Ok(BlockOutput::default())
  }
}

// used in unit tests only
#[cfg(test)]
impl Executable for u8 {
  fn execute(
    &self,
    _vm: &Machine,
    _state: &dyn State,
  ) -> Result<BlockOutput, MachineError> {
    Ok(BlockOutput::default())
  }
}
