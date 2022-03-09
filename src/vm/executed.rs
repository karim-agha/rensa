use {
  super::{machine::MachineError, output::BlockOutput, state::State, Machine},
  crate::consensus::{BlockData, Produced},
  std::{ops::Deref, sync::Arc},
};

/// Represents a block that has been executed by the virtual
/// machine along with the state changes that this execution
/// caused.
#[derive(Debug)]
pub struct Executed<D: BlockData> {
  pub underlying: Arc<Produced<D>>,
  pub output: Arc<BlockOutput>,
}

impl<D: BlockData> Clone for Executed<D> {
  fn clone(&self) -> Self {
    Self {
      underlying: Arc::clone(&self.underlying),
      output: Arc::clone(&self.output),
    }
  }
}

impl<D: BlockData> Executed<D> {
  pub fn new(
    state: &impl State,
    block: Arc<Produced<D>>,
    machine: &Machine,
  ) -> Result<Self, MachineError> {
    let output = Arc::new(machine.execute(state, &block)?);
    let underlying = block;

    if output.hash() == &underlying.state_hash {
      Ok(Self { output, underlying })
    } else {
      Err(MachineError::InconsistentStateHash)
    }
  }

  pub fn recreate(block: Produced<D>, output: BlockOutput) -> Self {
    Self {
      underlying: Arc::new(block),
      output: Arc::new(output),
    }
  }

  pub fn state(&self) -> &impl State {
    &self.output.state
  }
}

impl<D: BlockData> Deref for Executed<D> {
  type Target = Produced<D>;

  fn deref(&self) -> &Self::Target {
    &self.underlying
  }
}
