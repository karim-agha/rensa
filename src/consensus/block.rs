use super::{validator::Validator, vote::Vote};
use crate::keys::{Keypair, Pubkey};
use chrono::{DateTime, Utc};
use ed25519_dalek::{PublicKey, Signature, Signer, Verifier};
use multihash::{Code as MultihashCode, Multihash, MultihashDigest};
use serde::{Deserialize, Serialize};
use std::{
  fmt::Debug,
  io::{Error as StdIoError, ErrorKind},
  time::Duration,
};

/// The type requirements for anything that could be carried by a block
/// in the consensus layer. This is usually a list of transactions in
/// production settings, but it could also be strings or other types
/// for tests or specialized chains.
///
/// Essentially we need to be able to serialize and deserialize this data,
/// compare it for exact equality and print it in debug logs.
pub trait BlockData:
  Eq + Clone + Debug + Serialize + for<'a> Deserialize<'a>
{
}

/// Blanket implementation for all types that fulfill those requirements
impl<T> BlockData for T where
  T: Eq + Clone + Debug + Serialize + for<'a> Deserialize<'a>
{
}

/// Represents the type of values on which the consensus protocol
/// decides among many competing versions.
///
/// D is type of the underlying data that consensus is trying to
///   decide on, in case of a blockchain it is going to be Blocks
///
pub trait Block<D: BlockData>: Debug {
  /// Hash of this block with its payload.
  fn hash(&self) -> Result<Multihash, StdIoError>;

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
  fn data(&self) -> &D;

  /// All valid votes accumulated for this target block from other
  /// validators. A vote on a block is also implicitly a vote on
  /// all its ancestors.
  fn votes(&self) -> &[Vote];

  /// Serializes the contents of the block into a byte buffer.
  /// This serialization must be stable as it is
  /// used for calculating hashes.
  fn to_bytes(&self) -> Result<Vec<u8>, std::io::Error>;
}

/// The genesis block of the blockchain.
///
/// Defines the very first block of a chain with a fixed
/// set of validators and a few other settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
  bound = "D: Serialize, D: Eq, for<'a> D: Deserialize<'a>",
  rename_all = "camelCase"
)]
pub struct Genesis<D: BlockData> {
  /// The globally unique string that identifies this chain
  /// on the global network. This value is used to allow many
  /// instances of this validator software to be deployed as
  /// completely independent blockchains.
  pub chain_id: String,

  /// The hash function used in this blockchain for hasing
  /// blocks, transactions, and signatures.
  #[serde(with = "multihash_serde")]
  pub hasher: MultihashCode,

  /// The timepoint int UTC timestamp which specifies when
  /// the blockchain is due to start. At this time validators
  /// are supposed to come online and start participating in the
  /// consensus process. Slots and epochs times are calculated
  /// from this timepoint.
  pub genesis_time: DateTime<Utc>,

  /// Thr length of a single slot during which there is one
  /// leader validator that proposes new blocks. Regardless if
  /// the leader produces a new block during this slot or not,
  /// the consensus will advance to the next leader validator
  /// when the slot time elapses.
  #[serde(with = "humantime_serde")]
  pub slot_interval: Duration,

  /// How many slots make up one epoch. Epochs are groups of
  /// consecutive slots. Two epochs in a row that receive 2/3
  /// of validator votes constitute a finalized chechpoint that
  /// will never be reverted by any fork choice rule and could
  /// be considered forever immutable.
  pub epoch_slots: u64,

  /// The set of validators participating in the consensus along
  /// with their attributed stakes. Validators are always sorted
  /// so that the order of their appearance in the genesis file
  /// does not change the hash of the genesis.
  pub validators: Vec<Validator>,

  /// Block data stored in the first block.
  ///
  /// This is specific to the execution layer that is responsible
  /// for executing blocks and building state.
  pub data: D,
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
  pub parent: Multihash,

  /// The slot height at which it was produced.
  pub height: u64,

  /// The public key of the validator that produced this block
  /// along with a signature using their private key of the hash
  /// of this block.
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
}

impl<D: BlockData> Debug for Produced<D> {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("Produced")
      .field(
        "parent",
        &bs58::encode(self.parent.to_bytes()).into_string(),
      )
      .field("height", &self.height)
      .field("signature", &self.signature)
      .field("data", &self.data)
      .field("votes", &self.votes)
      .field(
        "hash",
        &bs58::encode(self.hash().unwrap().to_bytes()).into_string(),
      )
      .finish()
  }
}

impl<D: BlockData> Block<D> for Genesis<D> {
  /// The hash of the genesis is used to determine a
  /// unique fingerprint of a blockchain configuration.
  fn hash(&self) -> Result<Multihash, StdIoError> {
    // note: this could be optimized into zero-copy
    let buffer = self.to_bytes()?;
    Ok(self.hasher.digest(&buffer))
  }

  /// Always errors because this is the very first
  /// block of a block chain and its structure is
  /// special and different than other blocks produced
  /// by block proposers through the lifetime of the chain.
  fn parent(&self) -> Result<Multihash, StdIoError> {
    Err(StdIoError::new(
      ErrorKind::NotFound,
      "The genesis block has no parent",
    ))
  }

  /// The genesis block has no producer and thus nobody
  /// signed that block, as it comes from a config file
  /// rather than a validator.
  fn signature(&self) -> Option<&(Pubkey, Signature)> {
    None
  }

  /// Constant zero
  fn height(&self) -> u64 {
    0
  }

  /// The initial set of data stored in the genesis.
  /// This data is specific to the execution layer
  /// that drives the chain
  fn data(&self) -> &D {
    &self.data
  }

  /// The gensis block has no votes because it is a
  /// constant parameter to the validator during the
  /// process startup.
  fn votes(&self) -> &[Vote] {
    &[]
  }

  /// Serializes the contents of the block into a byte buffer.
  /// This serialization must be stable as it is
  /// used for calculating hashes.
  fn to_bytes(&self) -> Result<Vec<u8>, std::io::Error> {
    bincode::serialize(&self)
      .map_err(|e| std::io::Error::new(ErrorKind::InvalidData, e))
  }
}

impl<D: BlockData> Block<D> for Produced<D> {
  /// Hashes the contents of the current block using the
  /// same hashing algorithm that was used to hash its parent.
  /// This way it will recursively reuse the same hashing algo
  /// specified in the genesis block.
  ///
  /// This value is computed (as opposed to stored) intentionally
  /// to detect discrepancies between blocks and to avoid having
  /// to verify the correctness of the block hash.
  fn hash(&self) -> Result<Multihash, StdIoError> {
    let buffer = Self::hash_bytes(
      &self.signature.0,
      &self.height,
      &self.parent,
      &self.data,
      &self.votes,
    )?;

    // all hashes in a given chain are always produced
    // using the same hashing algorithm that was define
    // in the genesis. Read the algorithm code from
    // the parent block multihash and use it to hash
    // the current block contents.
    let hasher = MultihashCode::try_from(self.parent.code()).map_err(|e| {
      std::io::Error::new(
        ErrorKind::InvalidInput,
        format!("Parent block is hashed using unsupported algorithm: {e}"),
      )
    })?;

    Ok(hasher.digest(&buffer))
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

  /// The public key of the validator that produced this block
  /// and a signature that signs the hash of this block using
  /// producer's private key
  fn signature(&self) -> Option<&(Pubkey, Signature)> {
    Some(&self.signature)
  }

  /// The number of the time slot at which the block was produced.
  ///
  /// This is always a value greater than zero, and there may be gaps
  /// in the block height in the blockchain if a producer fails to
  /// produce a block during its turn.
  fn height(&self) -> u64 {
    self.height
  }

  /// The data carried by the block.
  /// Most often this is a list of transactions, unless some
  /// special variations are used for testing. The interpretation
  /// of the data is left to the execution layer.
  fn data(&self) -> &D {
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

  /// Serializes the contents of the block into a byte buffer.
  /// This serialization must be stable as it is
  /// used for calculating hashes.
  fn to_bytes(&self) -> Result<Vec<u8>, std::io::Error> {
    bincode::serialize(&self)
      .map_err(|e| std::io::Error::new(ErrorKind::InvalidData, e))
  }
}

impl<D: BlockData> Produced<D> {
  pub fn new(
    keypair: &Keypair,
    height: u64,
    parent: Multihash,
    data: D,
    votes: Vec<Vote>,
  ) -> Result<Self, std::io::Error> {
    let buffer =
      Self::hash_bytes(&keypair.public(), &height, &parent, &data, &votes)?;
    let hash = multihash::Code::try_from(parent.code())
      .map_err(|e| std::io::Error::new(ErrorKind::InvalidInput, e))?
      .digest(&buffer);
    let signature = (keypair.public(), (*keypair).sign(&hash.to_bytes()));

    Ok(Self {
      parent,
      height,
      signature,
      data,
      votes,
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
  fn hash_bytes(
    validator: &Pubkey,
    height: &u64,
    parent: &Multihash,
    data: &D,
    votes: &[Vote],
  ) -> Result<Vec<u8>, std::io::Error> {
    let mut buffer = Vec::new();
    buffer.append(&mut parent.to_bytes());
    buffer.append(&mut height.to_le_bytes().to_vec());
    buffer.append(
      &mut bincode::serialize(data)
        .map_err(|e| std::io::Error::new(ErrorKind::InvalidData, e))?,
    );
    buffer.append(
      &mut bincode::serialize(votes)
        .map_err(|e| std::io::Error::new(ErrorKind::InvalidData, e))?,
    );
    buffer.append(&mut validator.to_vec());
    Ok(buffer)
  }
}

impl<D: BlockData> std::fmt::Display for Produced<D> {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    let hash = self.hash().map_err(|_| std::fmt::Error)?;
    write!(
      f,
      "[{} @ {}]",
      bs58::encode(hash.to_bytes()).into_string(),
      self.height()
    )
  }
}

impl<D: BlockData> std::fmt::Display for Genesis<D> {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    let hash = self.hash().map_err(|_| std::fmt::Error)?;
    write!(
      f,
      "Genesis([{} @ {}])",
      bs58::encode(hash.to_bytes()).into_string(),
      self.height()
    )
  }
}

mod multihash_serde {
  use multihash::Code as MultihashCode;
  use serde::{
    de::{self, Visitor},
    Deserializer, Serializer,
  };

  pub fn serialize<S>(
    code: &MultihashCode,
    serializer: S,
  ) -> Result<S::Ok, S::Error>
  where
    S: Serializer,
  {
    let code: u64 = u64::from(*code);
    serializer.serialize_u64(code)
  }

  pub fn deserialize<'de, D>(deserializer: D) -> Result<MultihashCode, D::Error>
  where
    D: Deserializer<'de>,
  {
    /// In binary encoding we want to use the u64 format from the
    /// multicodec registry: https://github.com/multiformats/multicodec/blob/master/table.csv
    ///
    /// However in human readable formats, like JSON, we want users to be able
    /// to specify the hashing function using a human understandable string.
    struct NumberOrString;

    impl<'de> Visitor<'de> for NumberOrString {
      type Value = MultihashCode;

      fn expecting(
        &self,
        formatter: &mut std::fmt::Formatter,
      ) -> std::fmt::Result {
        formatter.write_str(concat!(
          "Multihash numeric code or hashing algorithm name. ",
          "See https://github.com/multiformats/multicodec/blob/master/table.csv"
        ))
      }

      fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
      where
        E: de::Error,
      {
        match value.to_lowercase().as_str() {
          "sha2-256" => Ok(MultihashCode::Sha2_256),
          "sha2-512" => Ok(MultihashCode::Sha2_512),
          "sha3-224" => Ok(MultihashCode::Sha3_224),
          "sha3-256" => Ok(MultihashCode::Sha3_256),
          "sha3-384" => Ok(MultihashCode::Sha3_384),
          "sha3-512" => Ok(MultihashCode::Sha3_512),
          "keccak-224" => Ok(MultihashCode::Keccak224),
          "keccak-256" => Ok(MultihashCode::Keccak256),
          "keccak-384" => Ok(MultihashCode::Keccak384),
          "keccak-512" => Ok(MultihashCode::Keccak512),
          "blake2b-256" => Ok(MultihashCode::Blake2b256),
          "blake2b-512" => Ok(MultihashCode::Blake2b512),
          "blake2s-256" => Ok(MultihashCode::Blake2s256),
          "blake3" => Ok(MultihashCode::Blake3_256),
          _ => Err(de::Error::custom(format!(
            "unrecognized hash algorithm type {value}"
          ))),
        }
      }

      fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
      where
        E: de::Error,
      {
        MultihashCode::try_from(v)
          .map_err(|e| de::Error::custom(format!("{e:?}")))
      }
    }

    deserializer.deserialize_any(NumberOrString)
  }
}
