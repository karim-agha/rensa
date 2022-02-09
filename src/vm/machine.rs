use {
  super::{
    builtin::BUILTIN_CONTRACTS,
    contract::{ContractEntrypoint, Output},
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
  tracing::{trace, warn},
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

  #[error("Signature verification failed")]
  SignatureVerificationFailed,

  #[error("Not all accounts declared as signers have signatures")]
  MissingSigners,

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
        match transaction.create_environment(&state) {
          Ok(env) => {
            match entrypoint(env, &transaction.params) {
              Ok(outputs) => {
                let txstate =
                  outputs.into_iter().fold(StateDiff::default(), |s, o| {
                    s.merge(process_transaction_output(o, &state, transaction))
                  });
                accstate = accstate.merge(txstate);
              }
              Err(error) => {
                // when a transaction fails, none of its state changes
                // gets persisted. Todo: Add failure logs.
                warn!(
                  "transaction {} failed: {error}",
                  transaction.hash().to_b58()
                );
              }
            }
          }
          Err(error) => {
            warn!(
              "transaction {} failed: {error}",
              transaction.hash().to_b58()
            );
          }
        }
      } else {
        warn!(
          "transaction {} failed: unknown contract {}",
          transaction.hash().to_b58(),
          transaction.contract
        );
      }
    }
    Ok(accstate)
  }
}

fn process_transaction_output(
  output: Output,
  state: &impl State,
  transaction: &Transaction,
) -> StateDiff {
  let mut txstate = StateDiff::default();
  match output {
    Output::LogEntry(key, value) => {
      trace!(
        "transaction {} log: {key} => {value}",
        transaction.hash().to_b58()
      ); // todo
    }
    Output::ModifyAccountData(addr, data) => {
      trace!(
        "transaction {} modifying account {addr} with {data:?}",
        transaction.hash().to_b58()
      );
      modify_account_data(addr, data, state, &mut txstate, transaction);
    }
    Output::CreateOwnedAccount(addr, data) => {
      trace!(
        "transaction {} creating account {addr} with: {data:?}",
        transaction.hash().to_b58(),
      );
      create_owned_account(addr, data, state, &mut txstate, transaction);
    }
  };
  txstate
}

fn create_owned_account(
  address: Pubkey,
  data: Option<Vec<u8>>,
  state: &impl State,
  txstate: &mut impl State,
  transaction: &Transaction,
) {
  if state.get(&address).is_none() {
    set_state_if_writable(
      txstate,
      address,
      Account {
        executable: false,
        owner: Some(transaction.contract.clone()),
        data,
      },
      transaction,
    );
  }
}

fn modify_account_data(
  address: Pubkey,
  data: Option<Vec<u8>>,
  state: &impl State,
  txstate: &mut impl State,
  transaction: &Transaction,
) {
  if let Some(acc) = state.get(&address) {
    set_state_if_writable(
      txstate,
      address,
      Account {
        data,
        ..acc.clone()
      },
      transaction,
    );
  }
}

fn set_state_if_writable(
  state: &mut impl State,
  addr: Pubkey,
  value: Account,
  transaction: &Transaction,
) {
  for acc in &transaction.accounts {
    if acc.address == addr && acc.writable {
      state.set(addr, value).unwrap();
      break;
    }
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
