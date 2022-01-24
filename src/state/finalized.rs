use super::{Result, State, StateDiff, StateError};
use crate::primitives::{Account, Pubkey};
use multihash::Multihash;

/// Represents state of the blockchain at the last finalized
/// block. This state is persisted to disk and is not affected
/// by blockchain forks in the consensus.
///
/// Data in this state is large (counted in GBs). It gets updated
/// by applying StateDiffs to it from newly finalized blocks.
pub struct FinalizedState;

impl FinalizedState {
  pub fn _apply(&mut self, _diff: StateDiff) -> Result<()> {
    todo!();
  }
}

impl State for FinalizedState {
  fn get(&self, _address: &Pubkey) -> Option<&Account> {
    todo!()
  }

  /// Writes directly to finalized state are not supported, instead
  /// state diffs from newly finalized blocks should be applied using the
  /// [`apply`] method
  fn set(&mut self, _: Pubkey, _: Account) -> Result<Option<Account>> {
    Err(StateError::WritesNotSupported)
  }

  /// The data hash of the entire finalized state.
  ///
  /// This field is simlar in its purpose to a merkle tree in ethereum,
  /// except it also represents valid IPFS CIDv1 PB-DAG entries, that can
  /// be used to sync blockchain state up to this point from other peers
  /// or from external IPFS pinning services.
  fn hash(&self) -> Multihash {
    todo!()
  }
}
