use super::{
  block::{self, Block, BlockData},
  validator::Validator,
  vote::Vote,
};
use crate::keys::Pubkey;
use dashmap::DashMap;
use multihash::Multihash;
use tokio::sync::RwLock;
use tracing::{info, warn};

struct TreeNode<'b, D: BlockData> {
  value: &'b block::Produced<D>,
  parent: Option<&'b block::Produced<D>>,
  children: Vec<&'b block::Produced<D>>,
}

struct ForkTree<'b, D: BlockData> {
  root: TreeNode<'b, D>,
}

impl<'b, D: BlockData> ForkTree<'b, D> {
  pub fn new(root: &'b block::Produced<D>) -> Self {
    Self {
      root: TreeNode {
        value: root,
        parent: None,
        children: vec![],
      },
    }
  }
}

/// A block that is still not finalized and its votes
/// are still being counted.
///
/// Those blocks are not guaranteed to never be
/// discarded by the blockchain yet.
struct VolatileBlock<D: BlockData> {
  block: block::Produced<D>,
  votes: u64,
}

/// State of the unfinalized part of the chain.
/// Blocks in this state can be overriden using
/// fork choice rules and voting.
struct VolatileState<'b, D: BlockData> {
  pending: Vec<VolatileBlock<D>>,
  forktree: Option<ForkTree<'b, D>>,
}

impl<'b, D: BlockData> VolatileState<'b, D> {
  pub fn new() -> Self {
    Self {
      pending: vec![],
      forktree: None,
    }
  }
}

/// Represents the state of the consensus protocol
pub struct Chain<'g, 'b, D: BlockData> {
  genesis: &'g block::Genesis<D>,
  stakes: DashMap<Pubkey, u64>,
  finalized: Vec<block::Produced<D>>,
  volatile: RwLock<VolatileState<'b, D>>,
}

impl<'g, 'b, D: BlockData> Chain<'g, 'b, D> {
  pub fn new(genesis: &'g block::Genesis<D>) -> Self {
    Self {
      genesis,
      finalized: vec![],
      volatile: RwLock::new(VolatileState::new()),
      stakes: genesis
        .validators
        .iter()
        .map(|v| (v.pubkey.clone(), v.stake))
        .collect(),
    }
  }

  /// Returns the very first block of the blockchain
  /// that contains initial state setup and various
  /// chain configurations.
  pub fn genesis(&self) -> &'g block::Genesis<D> {
    self.genesis
  }

  /// Returns the hash of the last finalized block in the chain.
  /// Blocks that reached finality will never be reverted under
  /// any circumstances.
  /// 
  /// If no block has been finalized yet, then the genesis block
  /// hash is used as the last finalized block.
  ///
  /// This value is used as the justification when voting for new
  /// blocks, also the last finalized block is the root of the
  /// current fork tree.
  pub fn finalized(&self) -> Multihash {
    match self.finalized.last() {
      Some(b) => b
        .hash()
        .expect("a block with invalid hash would not get finalized"),
      None => self
        .genesis
        .hash()
        .expect("invalid genesis hash would have crashed the system already"),
    }
  }

  /// Represents the current set of validators that are
  /// taking part in the consensus. For now, this value
  /// is static and based on what is defined in genesis.
  ///
  /// In the next iteration validators will be able to
  /// join and leave the blockchain.
  pub fn validators(&self) -> &'g [Validator] {
    &self.genesis.validators
  }

  /// The minimum voted stake that constitutes a 2/3 majority
  pub fn minimum_majority_stake(&self) -> u64 {
    let total_stake = self.validators().iter().fold(0, |a, v| a + v.stake);
    (total_stake as f64 * 0.67f64).ceil() as u64
  }
}

impl<'g, 'b, D: BlockData> Chain<'g, 'b, D> {
  pub async fn append(&self, block: block::Produced<D>) {
    if let Some(stake) = self.stakes.get(&block.proposer) {
      let mut unlocked = self.volatile.write().await;
      unlocked.pending.push(VolatileBlock {
        block,
        votes: *stake, // block proposition is counted as a vote on the block
      });
    } else {
      warn!(
        "Rejecting block from non-staking proposer {}",
        block.proposer
      );
    }
  }

  pub async fn vote(&self, vote: Vote) {
    let stake = match self.stakes.get(&vote.validator) {
      Some(stake) => *stake,
      None => {
        warn!("rejecting vote from unknown validator: {}", vote.validator);
        return;
      }
    };

    if vote.justification != self.finalized() {
      warn!(
        "Rejecting vote. Not justified by the last finalized block: {vote:?}"
      );
      return;
    }

    if !vote.verify_signature() {
      warn!("Rejecting vote {vote:?}. signature verification failed");
      return;
    }

    // todo:
    // let target = find_block in fork tree
    // add stake to block votes and all its ancestors
    // up to the justification point
    info!("recoding {vote:?} with stake {stake}");
  }
}
