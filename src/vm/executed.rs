use std::ops::Deref;

use super::StateDiff;
use crate::consensus::{BlockData, Produced};

/// Represents a block that has been executed by the virtual
/// machine along with the state changes that this execution
/// caused.
pub struct Executed<D: BlockData> {
  underlying: Produced<D>,
  _state_diff: StateDiff,
}

impl<D: BlockData> Deref for Executed<D> {
  type Target = Produced<D>;

  fn deref(&self) -> &Self::Target {
    &self.underlying
  }
}
