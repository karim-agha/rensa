use {
  super::vote::Vote,
  crate::{
    primitives::{Keypair, Pubkey, ToBase58String},
    vm::Executable,
  },
  ed25519_dalek::{PublicKey, Signature, Signer, Verifier},
  multihash::{
    Code as MultihashCode,
    Hasher,
    Multihash,
    MultihashDigest,
    Sha3_256,
  },
  once_cell::sync::OnceCell,
  serde::{Deserialize, Serialize},
  std::{
    any::Any,
    fmt::Debug,
    io::{Error as StdIoError, ErrorKind},
  },
};

/// The type requirements for anything that could be carried by a block
/// in the consensus layer. This is usually a list of transactions in
/// production settings, but it could also be strings or other types
/// for tests or specialized chains.
///
/// Essentially we need to be able to serialize and deserialize this data,
/// compare it for exact equality and print it in debug logs and have the
/// ability to execute it in a VM that supports the implemented type.
pub trait BlockData:
  Eq
  + Clone
  + Debug
  + Default
  + Executable
  + Serialize
  + Send
  + Sync
  + 'static
  + for<'a> Deserialize<'a>
{
  fn hash(&self) -> Result<Multihash, std::io::Error>;
}

/// Blanket implementation for all types that fulfill those requirements
impl<T> BlockData for T
where
  T: Eq
    + Clone
    + Debug
    + Default
    + Executable
    + Serialize
    + Send
    + Sync
    + 'static
    + for<'a> Deserialize<'a>,
{
  fn hash(&self) -> Result<Multihash, std::io::Error> {
    let mut sha3 = Sha3_256::default();
    sha3.update(
      // todo: make this zero-copy
      &bincode::serialize(self)
        .map_err(|e| std::io::Error::new(ErrorKind::InvalidData, e))?,
    );
    MultihashCode::Sha3_256
      .wrap(sha3.finalize())
      .map_err(|e| std::io::Error::new(ErrorKind::Other, e))
  }
}

/// Represents the type of values on which the consensus protocol
/// decides among many competing versions.
///
/// D is type of the underlying data that consensus is trying to
///   decide on, in case of a blockchain it is going to be Blocks
pub trait Block<D: BlockData>: Debug {
  /// Hash of this block with its payload.
  fn hash(&self) -> Result<Multihash, StdIoError>;

  /// Hash of the state diff resulting from executing the
  /// payload of this block. This value has two uses:
  ///  1. it is used as an IPFS CID for syncing latest blockchain state
  ///  2. is is used to verify that the result of block execution is
  ///     identical on every validator.
  fn state_hash(&self) -> Multihash;

  /// The previous block that this block builds
  /// off in the fork tree.
  fn parent(&self) -> Result<Multihash, StdIoError>;

  /// The public key of the validator that produced
  /// this block along with an Ed25519 signature of the hash
  /// of this block produced using validator's private key.
  ///
  /// The genesis block does not have a signature or
  /// a producer.
  fn signature(&self) -> Option<&(Pubkey, Signature)>;

  /// Slot height at which the block was produced.
  fn height(&self) -> u64;

  /// Block contents, that are opaque to the consensus.
  /// In most cases this is a list of transactions.
  fn payload(&self) -> &D;

  /// All valid votes accumulated for this target block from other
  /// validators. A vote on a block is also implicitly a vote on
  /// all its ancestors.
  fn votes(&self) -> &[Vote];

  /// Downcasting on block type
  fn as_any(&self) -> &dyn Any;
}

/// A block produced by one of the validators after Genesis.
///
/// A block of this type is at height at least 1 and is dynamically
/// appended to the chain by block producers and voted on by other
/// validators.
#[derive(Clone, Serialize, Deserialize)]
#[serde(
  bound = "D: Serialize, D: Eq, for<'a> D: Deserialize<'a>",
  rename_all = "camelCase"
)]
pub struct Produced<D: BlockData> {
  /// Hash of the parent block
  #[serde(with = "crate::primitives::b58::serde::multihash")]
  pub parent: Multihash,

  /// Hash of the state diff between this block and its parent.
  #[serde(with = "crate::primitives::b58::serde::multihash")]
  pub state_hash: Multihash,

  /// The height at which it was produced.
  pub height: u64,

  /// The public key of the validator that produced this block
  /// along with a signature using their private key of the hash
  /// of this block.
  #[serde(with = "crate::primitives::b58::serde::validator")]
  pub signature: (Pubkey, Signature),

  /// Block data stored in the block.
  ///
  /// This is specific to the execution layer that is responsible
  /// for executing blocks and building state. Usually this is a list
  /// of transactions.
  pub data: D,

  /// a list of signatures attesting to this block or previous blocks.
  /// a validator can sign any block link they want and the signature
  /// might arrive even few blocks late due to network latency or other
  /// factors.
  pub votes: Vec<Vote>,

  /// A cached version of the hash, once the hash is computed for the
  /// first time, its value is stored here, then retreived.
  #[serde(skip)]
  hashcahe: OnceCell<Multihash>,
}

impl<D: BlockData> Debug for Produced<D> {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("Produced")
      .field("parent", &self.parent.to_b58())
      .field("height", &self.height)
      .field(
        "signature",
        &format!(
          "Pubkey({}), ed25519({})",
          self.signature.0,
          self.signature.1.to_b58()
        ),
      )
      .field("data", &self.data)
      .field("votes", &self.votes)
      .field("hash", &self.hash().unwrap().to_b58())
      .field("state_hash", &self.state_hash().to_b58())
      .finish()
  }
}

impl<D: BlockData> std::hash::Hash for Produced<D> {
  fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
    <Produced<D> as Block<D>>::hash(self).unwrap().hash(state);
  }
}

impl<D: BlockData> PartialEq for Produced<D> {
  fn eq(&self, other: &Self) -> bool {
    self.hash().unwrap().eq(&other.hash().unwrap())
  }
}
impl<D: BlockData> Eq for Produced<D> {}

impl<D: BlockData> Block<D> for Produced<D> {
  /// Hashes of the current block.
  ///
  /// This value is computed (as opposed to stored) intentionally
  /// to detect discrepancies between blocks and to avoid having
  /// to verify the correctness of the block hash.
  fn hash(&self) -> Result<Multihash, StdIoError> {
    self
      .hashcahe
      .get_or_try_init(|| {
        Self::hash_parts(
          &self.signature.0,
          &self.height,
          &self.parent,
          &self.state_hash,
          &self.data,
          &self.votes,
        )
      })
      .map(|hash| *hash)
      .map_err(|e| std::io::Error::new(ErrorKind::InvalidData, e))
  }

  /// Hash of the first ancestor of this block.
  ///
  /// There may be more than one block with the same ancesor
  /// and that creates a fork in the blockchain that is resolved
  /// by the consensus algorithm.
  ///
  /// The hash is a multihash which means that the first bytes
  /// specify the code of the algorithm used to hash the data.
  ///
  /// The hashing algorithm is always reused in descendant blocks.
  fn parent(&self) -> Result<Multihash, StdIoError> {
    Ok(self.parent)
  }

  /// Hash of the state difference generated by executing the
  /// payload of this block on top of its parent's state.
  fn state_hash(&self) -> Multihash {
    self.state_hash
  }

  /// The public key of the validator that produced this block
  /// and a signature that signs the hash of this block using
  /// producer's private key
  fn signature(&self) -> Option<&(Pubkey, Signature)> {
    Some(&self.signature)
  }

  /// The height at which the block was produced.
  ///
  /// This is always a value greater than zero, and there are no gaps
  /// for this value, it is always parent height + 1
  fn height(&self) -> u64 {
    self.height
  }

  /// The data carried by the block.
  /// Most often this is a list of transactions, unless some
  /// special variations are used for testing. The interpretation
  /// of the data is left to the execution layer.
  fn payload(&self) -> &D {
    &self.data
  }

  /// Any votes accumulated during the production of this block.
  /// Those votes don't have to be for a specific block, and most
  /// likely they are for previous blocks as validators validate and
  /// propagate votes.
  ///
  /// Those votes are used to decide on the preferred fork in the
  /// Greedy Heaviest Observed Subtree (GHOST) fork choice algo.
  fn votes(&self) -> &[Vote] {
    &self.votes
  }

  fn as_any(&self) -> &dyn Any {
    self
  }
}

impl<D: BlockData> Produced<D> {
  pub fn new(
    keypair: &Keypair,
    height: u64,
    parent: Multihash,
    data: D,
    state_hash: Multihash,
    votes: Vec<Vote>,
  ) -> Result<Self, std::io::Error> {
    let signature = (
      keypair.public(),
      (*keypair).sign(
        &Self::hash_parts(
          &keypair.public(),
          &height,
          &parent,
          &state_hash,
          &data,
          &votes,
        )?
        .to_bytes(),
      ),
    );

    Ok(Self {
      parent,
      height,
      signature,
      data,
      state_hash,
      votes,
      hashcahe: OnceCell::new(),
    })
  }

  /// Verifies the validity of the signature of a block against
  /// the block hash as the message and the validator public key.
  pub fn verify_signature(&self) -> bool {
    if let Ok(msg) = self.hash() {
      if let Ok(pubkey) = PublicKey::from_bytes(&self.signature.0) {
        if let Ok(()) = pubkey.verify(&msg.to_bytes(), &self.signature.1) {
          return true;
        }
      }
    }
    false
  }

  /// Those are the bytes used to calculate block hash
  fn hash_parts(
    validator: &Pubkey,
    height: &u64,
    parent: &Multihash,
    state_hash: &Multihash,
    data: &D,
    votes: &[Vote],
  ) -> Result<Multihash, std::io::Error> {
    let mut sha3 = Sha3_256::default();
    sha3.update(validator);
    sha3.update(&parent.to_bytes());
    sha3.update(&state_hash.to_bytes());
    sha3.update(&height.to_le_bytes());
    sha3.update(&data.hash()?.to_bytes());
    for vote in votes {
      sha3.update(&vote.hash().to_bytes());
    }
    MultihashCode::Sha3_256
      .wrap(sha3.finalize())
      .map_err(|e| std::io::Error::new(ErrorKind::Other, e))
  }
}

impl<D: BlockData> std::fmt::Display for Produced<D> {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    let hash = self.hash().map_err(|_| std::fmt::Error)?;
    write!(f, "[{} @ {}]", hash.to_b58(), self.height())
  }
}
