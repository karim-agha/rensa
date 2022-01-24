use super::{State, StateDiff};
use crate::primitives::Transaction;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MachineError {
  #[error("Unknown error")]
  UnknownError,
}

type Result<T> = std::result::Result<T, MachineError>;

/// Represents a state machine that takes as an input a state
/// and a transaction and outputs a new state. This is the API
/// entry point to the virtual machine that runs contracts.
pub struct Machine;

impl Machine {
  pub fn execute(
    &self,
    _state: &impl State,
    _transaction: Transaction,
  ) -> Result<StateDiff> {
    todo!()
  }
}
