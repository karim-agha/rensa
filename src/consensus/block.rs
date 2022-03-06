use {
  super::{validator::Validator, vote::Vote},
  crate::{
    primitives::{Account, Keypair, Pubkey, ToBase58String},
    vm::{Executable, State, StateDiff},
  },
  chrono::{DateTime, Utc},
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
    collections::BTreeMap,
    fmt::Debug,
    io::{Error as StdIoError, ErrorKind},
    marker::PhantomData,
    time::Duration,
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

  /// Slot height at which the block was produced.
  fn slot(&self) -> u64;

  /// Block contents, that are opaque to the consensus.
  /// In most cases this is a list of transactions.
  fn payload(&self) -> &D;

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

  /// Maximum size of a block or any other single transmission
  /// over p2p gossip network in bytes.
  pub max_block_size: usize,

  /// How many blocks make up one epoch. Epochs are groups of
  /// consecutive slots. Two epochs in a row that receive 2/3
  /// of validator votes constitute a finalized chechpoint that
  /// will never be reverted by any fork choice rule and could
  /// be considered forever immutable.
  pub epoch_blocks: u64,

  /// The maximum age in epochs of a finalized block that can be
  /// used as a justification of a vote on a block. This value should
  /// be chosen in proportion to epoch slots and block time. If its too
  /// short then block propagation delays might invalidate votes, if it
  /// is too long, then it allows for long-range attacks and has higher
  /// memory footprint.
  pub max_justification_age: u64,

  /// The set of enabled builtin contracts.
  /// Builtins are special contracts implemented by the VM in
  /// native code. They are there for handling some computationally
  /// heavy operations like hashing or signature verification.
  ///
  /// The genesis block can enable those contracts by listing
  /// their pubkeys. See the [`vm/builtin`] module for more info.
  pub builtins: Vec<Pubkey>,

  /// The set of validators participating in the consensus along
  /// with their attributed stakes. Validators are always sorted
  /// so that the order of their appearance in the genesis file
  /// does not change the hash of the genesis.
  pub validators: Vec<Validator>,

  /// The minimum amount a validator has to stake to have its blocks
  /// accepted by the consensus. When a validator is offline for long
  /// enough, the penalties will start eating up its stake up to point
  /// where it drops below this level and then is excluded from
  /// consensus.
  pub minimum_stake: u64,

  /// The maximum number of accounts references a transaction accepts.
  /// This also means that this is the maximum number of distinct accounts
  /// a single transaction can interact with.
  pub max_input_accounts: usize,

  /// The initial accounts state of the chain at the very first block.
  /// This is a list of accounts along with their balances, owners and
  /// data. This is the very first finalized state in the chain before
  /// any produced block gets finalized.
  pub state: BTreeMap<Pubkey, Account>,

  /// Block data stored in the first block.
  ///
  /// This is specific to the execution layer that is responsible
  /// for executing blocks and building state.
  #[serde(skip)]
  pub _marker: PhantomData<D>,
}

impl<D: BlockData> Block<D> for Genesis<D> {
  /// The hash of the genesis is used to determine a
  /// unique fingerprint of a blockchain configuration.
  fn hash(&self) -> Result<Multihash, StdIoError> {
    let mut sha3 = Sha3_256::default();
    sha3.update(self.chain_id.as_bytes());
    sha3.update(&self.genesis_time.timestamp_millis().to_le_bytes());
    sha3.update(&self.slot_interval.as_millis().to_le_bytes());
    sha3.update(&self.epoch_blocks.to_le_bytes());
    sha3.update(&self.max_justification_age.to_le_bytes());
    sha3.update(&self.max_block_size.to_le_bytes());
    sha3.update(&self.minimum_stake.to_le_bytes());

    for builtin in &self.builtins {
      sha3.update(builtin);
    }

    for validator in &self.validators {
      sha3.update(&validator.pubkey);
      sha3.update(&validator.stake.to_le_bytes());
    }

    for (addr, acc) in &self.state {
      sha3.update(addr);
      match &acc.owner {
        Some(o) => sha3.update(o),
        None => sha3.update(&[0]),
      };

      match &acc.data {
        Some(v) => sha3.update(v),
        None => sha3.update(&[0]),
      }
    }

    MultihashCode::Sha3_256
      .wrap(sha3.finalize())
      .map_err(|e| std::io::Error::new(ErrorKind::Other, e))
  }

  /// Hash of the initial state configured in Genesis.
  fn state_hash(&self) -> Multihash {
    let mut state = StateDiff::default();
    for (k, v) in &self.state {
      state.set(*k, v.clone()).unwrap();
    }
    state.hash()
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

  /// Constant zero
  fn slot(&self) -> u64 {
    0
  }

  /// The initial set of data stored in the genesis.
  /// This data is specific to the execution layer
  /// that drives the chain
  fn payload(&self) -> &D {
    panic!("genesis block has no payload!");
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

  /// Hash of the state diff between this block and its parent.
  pub state_hash: Multihash,

  /// The slot at which it was produced.  
  pub slot: u64,

  /// The height at which it was produced.
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
          &self.slot,
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

  /// The time slot at which the block was produced.
  ///
  /// This is always a value greater than zero, and there may be gaps
  /// in the block height in the blockchain if a producer fails to
  /// produce a block during its turn.
  fn slot(&self) -> u64 {
    self.slot
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
    slot: u64,
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
          &slot,
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
      slot,
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
    slot: &u64,
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
    sha3.update(&slot.to_le_bytes());
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

impl<D: BlockData> std::fmt::Display for Genesis<D> {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    let hash = self.hash().map_err(|_| std::fmt::Error)?;
    write!(f, "Genesis([{} @ {}])", hash.to_b58(), self.height())
  }
}
