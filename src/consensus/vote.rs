use super::{block::BlockData, chain::Chain};
use crate::primitives::{Keypair, Pubkey, ToBase58String};
use ed25519_dalek::{PublicKey, Signature, Signer, Verifier};
use futures::Stream;
use multihash::{
  Code as MultihashCode, Multihash, MultihashDigest, Sha3_256, StatefulHasher,
};
use serde::{Deserialize, Serialize};
use std::{
  io::ErrorKind,
  marker::PhantomData,
  pin::Pin,
  task::{Context, Poll},
};
use tracing::warn;

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
#[derive(Clone, Serialize, Deserialize)]
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

impl std::fmt::Debug for Vote {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("Vote")
      .field("validator", &self.validator)
      .field("target", &self.target.to_b58())
      .field("justification", &self.justification.to_b58())
      .field("signature", &self.signature.to_b58())
      .finish()
  }
}

impl Vote {
  /// Verifies the signature of the vote.
  pub fn verify_signature(&self) -> bool {
    let mut msg = Vec::new();
    msg.append(&mut self.target.to_bytes());
    msg.append(&mut self.justification.to_bytes());
    match PublicKey::from_bytes(&self.validator) {
      Ok(p) => match p.verify(&msg, &self.signature) {
        Ok(_) => true,
        Err(e) => {
          warn!(
            "signature verification for vote from {} failed {e}",
            self.validator
          );
          false
        }
      },
      Err(e) => {
        warn!("invalid public key {}: {e}", self.validator);
        false
      }
    }
  }

  /// Creates new vote for a target block using validator's secret key.
  /// The justification must be the hash of the last finalized block on
  /// the chain. If no blocks are finalized yet, then the genesis block
  /// is considered as the last finalized.
  pub fn new(
    keypair: &Keypair,
    target: Multihash,
    justification: Multihash,
  ) -> Self {
    let mut msg = Vec::new();
    msg.append(&mut target.to_bytes());
    msg.append(&mut justification.to_bytes());
    let signature = (*keypair).sign(&msg);
    Self {
      validator: keypair.public(),
      target,
      justification,
      signature,
    }
  }

  pub fn to_bytes(&self) -> Result<Vec<u8>, std::io::Error> {
    bincode::serialize(&self)
      .map_err(|e| std::io::Error::new(ErrorKind::InvalidData, e))
  }

  pub fn hash(&self) -> Multihash {
    let mut sha3 = Sha3_256::default();
    sha3.update(&self.validator);
    sha3.update(&self.target.to_bytes());
    sha3.update(&self.justification.to_bytes());
    sha3.update(&self.signature.to_bytes());
    MultihashCode::multihash_from_digest(&sha3.finalize())
  }
}

pub struct VoteProducer<D: BlockData>(PhantomData<D>);

impl<D: BlockData> VoteProducer<D> {
  pub fn new(_chain: &Chain<D>) -> Self {
    VoteProducer(PhantomData)
  }
}

impl<D: BlockData> Stream for VoteProducer<D> {
  type Item = Vote;

  fn poll_next(
    self: Pin<&mut Self>,
    _: &mut Context<'_>,
  ) -> Poll<Option<Self::Item>> {
    Poll::Pending
  }
}
