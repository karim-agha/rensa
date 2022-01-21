use super::{
  block::{self, Block, BlockData},
  validator::Validator,
  vote::Vote,
};
use crate::keys::Pubkey;
use dashmap::DashMap;
use multihash::Multihash;
use std::rc::Rc;
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::warn;

struct TreeNode<D: BlockData> {
  value: VolatileBlock<D>,
  parent: Option<Rc<TreeNode<D>>>,
  children: Vec<Rc<TreeNode<D>>>,
}

impl<D: BlockData> TreeNode<D> {
  pub fn get_block_mut(
    self: &Rc<Self>,
    hash: &Multihash,
  ) -> Result<Option<Rc<Self>>, std::io::Error> {
    if self.value.block.hash()? == *hash {
      return Ok(Some(Rc::clone(self)));
    } else {
      for child in self.children.iter() {
        if let Some(ref b) = child.get_block_mut(hash)? {
          return Ok(Some(Rc::clone(b)));
        }
      }
      return Ok(None);
    }
  }

  pub fn add_child(
    mut self: Rc<Self>,
    block: VolatileBlock<D>,
  ) -> Result<Rc<TreeNode<D>>, std::io::Error> {
    let blockhash = block.block.hash()?;
    assert!(block.block.parent()? == self.value.block.hash()?);

    for child in self.children.iter() {
      if child.value.block.hash()? == blockhash {
        // block already a child of this block
        return Ok(Rc::clone(child));
      }
    }

    // set parent link and wrap in treenode
    let block = Rc::new(TreeNode {
      value: block,
      parent: Some(Rc::clone(&self)),
      children: vec![],
    });

    // store
    Rc::get_mut(&mut self)
      .unwrap()
      .children
      .push(Rc::clone(&block));

    Ok(block)
  }

  pub fn add_votes(mut self: Rc<Self>, votes: u64) {
    // apply those votes to the current block
    Rc::get_mut(&mut self).unwrap().value.votes += votes;

    // also apply those votes to all the parent votes
    // until the justification point.
    let mut current = Rc::clone(&self);
    while let Some(ancestor) =
      Rc::get_mut(&mut current).unwrap().parent.as_mut()
    {
      Rc::get_mut(ancestor).unwrap().value.votes += votes;
      current = Rc::clone(ancestor);
    }
  }
}

struct ForkTree<D: BlockData> {
  root: Rc<TreeNode<D>>,
}

impl<D: BlockData> ForkTree<D> {
  pub fn new(root: VolatileBlock<D>) -> Self {
    Self {
      root: Rc::new(TreeNode {
        value: root,
        parent: None,
        children: vec![],
      }),
    }
  }

  pub fn get_block_mut(&self, hash: &Multihash) -> Option<Rc<TreeNode<D>>> {
    self.root.get_block_mut(hash).unwrap_or(None)
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
struct VolatileState<D: BlockData> {
  root: Multihash,
  forktree: Vec<ForkTree<D>>,
}

impl<D: BlockData> VolatileState<D> {
  pub fn new(root: Multihash) -> Self {
    Self {
      root,
      forktree: vec![],
    }
  }

  /// Inserts a block into the current unfinalized fork tree.
  ///
  /// The stake parameter is the stake of the block proposer. We're
  /// treating block proposal as an implicit vote on it.
  pub fn append(
    &mut self,
    block: block::Produced<D>,
    stake: u64,
  ) -> Result<(), VolatileStateError> {
    let block = VolatileBlock {
      block,
      votes: stake, // block proposition is counted as a vote on the block
    };

    let parent = block.block.parent()?;

    // if we're the first block in the volatile state
    if self.forktree.is_empty() {
      if parent == self.root {
        self.forktree.push(ForkTree::new(block));
        return Ok(());
      } else {
        warn!(
          "rejecting block. cannot find parent {}",
          bs58::encode(parent.to_bytes()).into_string()
        );
        return Err(VolatileStateError::ParentBlockNotFound);
      }
    } else {
      // we already have some unfinalized blocks
      for root in self.forktree.iter_mut() {
        if let Some(b) = root.get_block_mut(&parent) {
          b.add_child(block)?;
          return Ok(());
        }
      }
      return Err(VolatileStateError::ParentBlockNotFound);
    }
  }

  pub fn vote(
    &mut self,
    vote: Vote,
    stake: u64,
  ) -> Result<(), VolatileStateError> {
    if vote.justification != self.root {
      warn!("Not justified by the last finalized block: {vote:?}");
      return Err(VolatileStateError::InvalidJustification);
    }

    for root in self.forktree.iter_mut() {
      if let Some(b) = root.get_block_mut(&vote.target) {
        b.add_votes(stake);
        return Ok(())
      }
    }

    Err(VolatileStateError::VoteTargetNotFound)
  }
}

#[derive(Debug, Error)]
enum VolatileStateError {
  #[error("Block's parent hash is invalid: {0}")]
  InvalidParentBockHash(#[from] std::io::Error),

  #[error("Parent block not found")]
  ParentBlockNotFound,

  #[error("Invalid justification for vote")]
  InvalidJustification,

  #[error("The block hash being voted on is not found")]
  VoteTargetNotFound,
}

/// Represents the state of the consensus protocol
pub struct Chain<'g, D: BlockData> {
  genesis: &'g block::Genesis<D>,
  stakes: DashMap<Pubkey, u64>,
  finalized: Vec<block::Produced<D>>,
  volatile: RwLock<VolatileState<D>>,
}

impl<'g, D: BlockData> Chain<'g, D> {
  pub fn new(genesis: &'g block::Genesis<D>) -> Self {
    Self {
      genesis,
      finalized: vec![],
      volatile: RwLock::new(VolatileState::new(genesis.hash().unwrap())),
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

impl<'g, D: BlockData> Chain<'g, D> {
  /// Called whenever a new block is received on the p2p layer.
  ///
  /// This method will validate signatures on the block and attempt
  /// to insert it into the volatile state of the chain.
  pub async fn append(&self, block: block::Produced<D>) {
    if let Some(stake) = self.stakes.get(&block.proposer) {
      let mut unlocked = self.volatile.write().await;
      if let Err(e) = unlocked.append(block, *stake) {
        warn!("block rejected: {e}");
      }
    } else {
      warn!(
        "Rejecting block from non-staking proposer {}",
        block.proposer
      );
    }
  }

  /// Called whenever a new vote is received on the p2p layer.
  ///
  /// This method will validate signatures on the vote and attempt
  /// to insert it into the volatile state of the chain.
  pub async fn vote(&self, vote: Vote) {
    let stake = match self.stakes.get(&vote.validator) {
      Some(stake) => *stake,
      None => {
        warn!("rejecting vote from unknown validator: {}", vote.validator);
        return;
      }
    };

    if !vote.verify_signature() {
      warn!("Rejecting vote {vote:?}. Signature verification failed");
      return;
    }

    let mut unlocked = self.volatile.write().await;
    if let Err(e) = unlocked.vote(vote, stake) {
      warn!("vote rejected: {e}");
    }
  }
}
