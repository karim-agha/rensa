use super::validator::Validator;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// The genesis block of the blockchain.
///
/// Defines the very first block of a chain with a fixed
/// set of validators and a few other settings.
#[derive(Debug, Serialize, Deserialize)]
#[serde(
  bound = "D: Serialize, for<'a> D: Deserialize<'a>",
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

  /// State of the first block, specific to the execution layer
  /// that is responsible for executing blocks.
  pub state: D,
}
