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
use {
  crate::primitives::{Keypair, Pubkey, ToBase58String},
  ed25519_dalek::{PublicKey, Signature, SignatureError, Signer, Verifier},
  multihash::{
    Code as MultihashCode,
    Hasher,
    Multihash,
    MultihashDigest,
    Sha3_256,
  },
  serde::{Deserialize, Serialize},
};

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
  #[serde(with = "crate::primitives::b58::serde::multihash")]
  pub target: Multihash,

  /// The hash of the last finalized block that is an
  /// ancestor of the [`target`]. See the finalization
  /// rules for more info.
  #[serde(with = "crate::primitives::b58::serde::multihash")]
  pub justification: Multihash,

  /// ED25519 signature using validator's private key.
  ///
  /// The message being signed is a concatinated bytestring
  /// of target bytes and justification bytes.
  #[serde(with = "crate::primitives::b58::serde::signature")]
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
  pub fn verify_signature(&self) -> Result<(), SignatureError> {
    let mut msg = Vec::new();
    msg.append(&mut self.target.to_bytes());
    msg.append(&mut self.justification.to_bytes());
    PublicKey::from_bytes(&self.validator)?.verify(&msg, &self.signature)
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

  pub fn hash(&self) -> Multihash {
    let mut sha3 = Sha3_256::default();
    sha3.update(&self.validator);
    sha3.update(&self.target.to_bytes());
    sha3.update(&self.justification.to_bytes());
    sha3.update(&self.signature.to_bytes());
    MultihashCode::Sha3_256.wrap(sha3.finalize()).unwrap()
  }
}
