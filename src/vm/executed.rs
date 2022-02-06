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
    machine: &Machine<D>,
  ) -> Result<Self, MachineError> {
    Ok(Self {
      state_diff: machine.execute(state, &block)?,
      underlying: block,
    })
  }

  pub fn state(&self) -> &impl State {
    &self.state_diff
  }

  pub fn take(self) -> Produced<D> {
    self.underlying
  }
}

impl<D: BlockData> Deref for Executed<D> {
  type Target = Produced<D>;

  fn deref(&self) -> &Self::Target {
    &self.underlying
  }
}
