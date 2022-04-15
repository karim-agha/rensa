use {
  super::{
    builtin::BUILTIN_CONTRACTS,
    contract::{ContractEntrypoint, ContractError, NativeContractEntrypoint},
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
    vm::{contract::Environment, runtime::Runtime, WASM_VM_BUILTIN_ADDR},
  },
  std::{cmp::Ordering, collections::HashMap},
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

  #[error("Transactions are not correctly ordered in this block")]
  InvalidTransactionsOrder,

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
  builtins: HashMap<Pubkey, NativeContractEntrypoint>,
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
  pub fn builtin(&self, addr: &Pubkey) -> Option<NativeContractEntrypoint> {
    self.builtins.get(addr).cloned()
  }

  /// Gets a WASM contract deployed externally to the blockchain.
  pub fn contract(
    &self,
    addr: &Pubkey,
    state: &dyn State,
  ) -> Result<ContractEntrypoint, ContractError> {
    if let Some(account) = state.get(addr) {
      if let Some(owner) = account.owner {
        if owner == *WASM_VM_BUILTIN_ADDR
          && account.executable
          && account.data.is_some()
        {
          let runtime = Runtime::new(account.data.unwrap().as_ref())?;
          return Ok(Box::new(move |env: &Environment, params: &[u8]| {
            runtime.invoke(env, params)
          }));
        }
      }

      Err(ContractError::AccountIsNotExecutable)
    } else {
      Err(ContractError::ContractDoesNotExit)
    }
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
    // transactions order within a block must follow a known
    // ordering algorithm described in more detail in the block
    // producer module. Reject all blocks that don't follow that
    // algorithm.
    if !verify_transactions_order(self) {
      return Err(MachineError::InvalidTransactionsOrder);
    }

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

/// All transactions must be sorted by their hashes (this makes some MEV attacks
/// very difficult). With the exception of transactions coming from the same
/// payer and having a monothonically increasing nonce
fn verify_transactions_order(txs: &[Transaction]) -> bool {
  if let Some(first) = txs.first() {
    let is_within_payer_group = |tx: &Transaction, prev: &Transaction| {
      tx.payer == prev.payer && tx.nonce == prev.nonce - 1
    };

    let mut prev = first;

    // the first transaction in a payer group
    let mut group_head = None;

    for tx in txs.iter().skip(1) {
      if !is_within_payer_group(tx, prev) {
        group_head = Some(tx);
      }

      if let Some(ghead) = group_head {
        if let Ordering::Less = tx.hash().cmp(ghead.hash()) {
          return false;
        }
      }

      prev = tx;
    }

    true // all txs sorted by hash
  } else {
    true // empty block
  }
}
