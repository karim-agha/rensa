use super::{State, StateDiff, Transaction};
use crate::consensus::block;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MachineError {
  #[error("Unknown error")]
  UnknownError,
}

/// Represents a state machine that takes as an input a state
/// and a block and outputs a new state. This is the API
/// entry point to the virtual machine that runs contracts.
pub struct Machine;

impl Machine {
  pub fn execute(
    &self,
    _state: &impl State,
    _block: block::Produced<Vec<Transaction>>,
  ) -> Result<StateDiff, MachineError> {
    todo!()
  }
}
