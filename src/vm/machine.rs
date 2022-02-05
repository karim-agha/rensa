use {
  super::{State, StateDiff, Transaction},
  crate::consensus::{BlockData, Genesis, Produced},
  thiserror::Error,
};

#[derive(Debug, Error)]
pub enum MachineError {
  #[error("Unknown error")]
  UnknownError,
}

pub trait Executable {
  fn execute(&self, state: &impl State) -> Result<StateDiff, MachineError>;
}

/// Represents a state machine that takes as an input a state
/// and a block and outputs a new state. This is the API
/// entry point to the virtual machine that runs contracts.
pub struct Machine<'g, D: BlockData> {
  _genesis: &'g Genesis<D>,
}

impl<'g, D: BlockData> Machine<'g, D> {
  pub fn new(_genesis: &'g Genesis<D>) -> Self {
    Self { _genesis }
  }

  pub fn execute(
    &self,
    state: &impl State,
    block: &Produced<D>,
  ) -> Result<StateDiff, MachineError> {
    block.data.execute(state)
  }
}

/// An implementation for blocks that carry a list of transactions.
impl Executable for Vec<Transaction> {
  fn execute(&self, _state: &impl State) -> Result<StateDiff, MachineError> {
    todo!()
  }
}

// used in unit tests only
#[cfg(test)]
impl Executable for String {
  fn execute(&self, _state: &impl State) -> Result<StateDiff, MachineError> {
    Ok(StateDiff::default())
  }
}

// used in unit tests only
#[cfg(test)]
impl Executable for u8 {
  fn execute(&self, _state: &impl State) -> Result<StateDiff, MachineError> {
    Ok(StateDiff::default())
  }
}
