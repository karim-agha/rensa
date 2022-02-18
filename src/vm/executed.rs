use {
  super::{
    machine::MachineError,
    state::{State, StateDiff},
    Machine,
  },
  crate::consensus::{BlockData, Produced},
  std::ops::Deref,
};

/// Represents a block that has been executed by the virtual
/// machine along with the state changes that this execution
/// caused.
#[derive(Debug, Clone)]
pub struct Executed<D: BlockData> {
  pub underlying: Produced<D>,
  pub state_diff: StateDiff,
}

impl<D: BlockData> Executed<D> {
  pub fn new(
    state: &impl State,
    block: Produced<D>,
    machine: &Machine,
  ) -> Result<Self, MachineError> {
    let state_diff = machine.execute(state, &block)?;
    let underlying = block;

    if state_diff.hash() == underlying.state_hash {
      Ok(Self {
        state_diff,
        underlying,
      })
    } else {
      Err(MachineError::InconsistentStateHash)
    }
  }

  pub fn state(&self) -> &impl State {
    &self.state_diff
  }
}

impl<D: BlockData> Deref for Executed<D> {
  type Target = Produced<D>;

  fn deref(&self) -> &Self::Target {
    &self.underlying
  }
}
