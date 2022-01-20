use crate::keys::Pubkey;
use ed25519_dalek::Signature;
use multihash::Multihash;
use serde::{Deserialize, Serialize};

// vote = (
//  validator,
//  target_block_hash,
//  target_epoch,    // epoch height
//  source_epoch     // epoch height, has to be justified
// )

// finalized checkpoint when two we have two
// justified (2/3 majority votes) checkpoints in a row

// slashing conditions:
//
// 1. No two votes from the same validator must have the same
//    target epoch.
//
// 2. no surround vote.
//      +----------> [h(s1) = 3] ----> [h(t1) = 4] --->
//  [J] +
//      +---> [h(s2) = 2]--------------------------> [h(t2) = 5] ---->

/// A message of this type means that a validator with the
/// public key [`validator`] is voting on the validity and
/// choice of a block with hash [`target], and justifies
/// that vote with a finalized block [`justification`].
///
/// The vote is signed using validator's public key over
/// bytes of [`target`] and [`justification`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vote {
  /// The public key of the validator casting a vote.
  pub validator: Pubkey,

  /// The hash of the block that is being voted on.
  /// A vote on a target block is implicitly also a
  /// vote on all blocks that are this target's
  /// ancestors until the justification block.
  pub target: Multihash,

  /// The hash of the last finalized block that is an
  /// ancestor of the [`target`]. See the finalization
  /// rules for more info.
  pub justification: Multihash,

  /// ED25519 signature using validator's private key.
  ///
  /// The message being signed is a concatinated bytestring
  /// of target bytes and justification bytes.
  pub signature: Signature,
}