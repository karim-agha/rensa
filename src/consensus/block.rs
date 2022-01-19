use super::validator::Validator;
use crate::keys::Pubkey;
use chrono::{DateTime, Utc};
use multihash::{Code as MultihashCode, Multihash};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Represents the type of values on which the consensus protocol
/// decides among many competing versions.
///
/// Type parameters:
/// D is type of the underlying data that consensus is trying to
///   decide on, in case of a blockchain it is going to be Blocks
///
/// S is type of the signatures gathered by the conensus to vote
///   and justify blocks in the fork tree.
///
pub trait Block<D, S>: Eq + Serialize + for<'a> Deserialize<'a>
where
  D: Eq + Serialize + for<'a> Deserialize<'a>,
  S: Eq + Serialize + for<'a> Deserialize<'a>,
{
  /// Hash of this block with its payload.
  fn hash(&self) -> Multihash;

  /// The previous block that this block builds
  /// off in the fork tree.
  fn parent(&self) -> Multihash;

  /// Slot height at which the block was produced.
  fn height(&self) -> u64;

  /// Block contents, that are opaque to the consensus.
  /// In most cases this is a list of transactions.
  fn data(&self) -> &D;

  /// BLS signature aggregates collected in this block for any
  /// previous blocks in the fork tree.
  fn signatures(&self) -> &[S];
}

/// The genesis block of the blockchain.
///
/// Defines the very first block of a chain with a fixed
/// set of validators and a few other settings.
#[derive(Debug, Serialize, Deserialize)]
#[serde(
  bound = "D: Serialize, D: Eq, for<'a> D: Deserialize<'a>",
  rename_all = "camelCase"
)]
pub struct Genesis<D>
where
  D: Eq + Serialize + for<'a> Deserialize<'a>,
{
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
  /// with their attributed stakes.
  pub validators: Vec<Validator>,

  /// Block data stored in the first block.
  ///
  /// This is specific to the execution layer that is responsible
  /// for executing blocks and building state.
  pub data: D,
}

/// A block produced by one of the validators.
///
/// A block of this type is at height at least 1 and is dynamically
/// appended to the chain by block producers.
pub struct Produced<D, S>
where
  D: Eq + Serialize + for<'a> Deserialize<'a>,
  S: Eq + Serialize + for<'a> Deserialize<'a>,
{
  /// The validator that proposed this block
  pub proposer: Pubkey,

  /// Hash of the parent block
  pub parent: Multihash,

  /// The slot height at which it was produced.
  pub height: u64,

  /// Block data stored in the block.
  ///
  /// This is specific to the execution layer that is responsible
  /// for executing blocks and building state. Usually this is a list
  /// of transactions.
  pub data: D,

  /// a list of signatures attesting to this block or previous blocks.
  /// a validator can sign any block link they want and the signature
  /// might arrive even few blocks late due to network latency.
  pub signatures: Vec<S>,
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
          "sha2-512" => Ok(MultihashCode::Sha2_256),
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
          _ => {
            Err(de::Error::custom(format!("unrecognized hash type {value}")))
          }
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
