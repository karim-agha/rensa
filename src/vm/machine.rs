use {
  super::{State, StateDiff, Transaction},
  crate::consensus::{BlockData, Produced},
  std::marker::PhantomData,
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
pub struct Machine<D: BlockData>(PhantomData<D>);

impl<D: BlockData> Default for Machine<D> {
  fn default() -> Self {
    Self(PhantomData)
  }
}

impl<D: BlockData> Machine<D> {
  pub fn execute(
    &self,
    state: &impl State,
    block: &Produced<D>,
  ) -> Result<StateDiff, MachineError> {
    block.data.execute(state)
  }
}

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
