use {
  super::{machine::MachineError, Machine, State, StateDiff},
  crate::consensus::{BlockData, Produced},
  std::ops::Deref,
};

/// Represents a block that has been executed by the virtual
/// machine along with the state changes that this execution
/// caused.
pub struct Executed<D: BlockData> {
  underlying: Produced<D>,
  _state_diff: StateDiff,
}

impl<D: BlockData> Executed<D> {
  pub fn _new(
    state: &impl State,
    block: Produced<D>,
    machine: &Machine<D>,
  ) -> Result<Self, MachineError> {
    Ok(Self {
      _state_diff: machine.execute(state, &block)?,
      underlying: block,
    })
  }

  pub fn _state(&self) -> &impl State {
    &self._state_diff
  }
}

impl<D: BlockData> Deref for Executed<D> {
  type Target = Produced<D>;

  fn deref(&self) -> &Self::Target {
    &self.underlying
  }
}
