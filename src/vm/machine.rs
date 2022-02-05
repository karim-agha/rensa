use std::marker::PhantomData;

use thiserror::Error;

use super::{State, StateDiff, Transaction};
use crate::consensus::{BlockData, Produced};

#[derive(Debug, Error)]
pub enum MachineError {
  #[error("Unknown error")]
  UnknownError,
}

/// Represents a state machine that takes as an input a state
/// and a block and outputs a new state. This is the API
/// entry point to the virtual machine that runs contracts.
pub struct Machine<D: BlockData>(PhantomData<D>);

impl<D: BlockData> Default for Machine<D> {
  fn default() -> Self {
    Self(PhantomData)
  }
}

impl<D: BlockData> Machine<D> {
  pub fn execute(
    &self,
    _state: &impl State,
    _block: Produced<Vec<Transaction>>,
  ) -> Result<StateDiff, MachineError> {
    todo!()
  }
}
