mod diff;
mod machine;

use crate::primitives::{Account, Pubkey};
use libp2p::multihash::Multihash;

pub use diff::StateDiff;
pub use machine::Machine;

/// Represents the state of the blockchain that is the result
/// of running the replicated state machine.
pub trait State {
  /// Retreives the account object and its contents from the state.
  fn get(&self, address: &Pubkey) -> Option<&Account>;

  /// Stores or overwrites an account object and its contents in the state.
  fn set(&mut self, address: &Pubkey, account: Account);

  /// Returns the CID or hash of the current state.
  ///
  /// Those CIDs are valid IPFS cids that can also be used
  /// for syncing blockchain state. Each State hash is a PB-DAG
  /// object that also points to the previous state that this
  /// state was built upon.
  fn hash(&self) -> Multihash;
}
