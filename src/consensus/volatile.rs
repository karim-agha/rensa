use super::{
  block::{self, Block, BlockData},
  vote::Vote,
};
use crate::keys::Pubkey;
use multihash::Multihash;
use std::{cell::RefCell, cmp::Ordering, collections::HashSet, rc::Rc};
use thiserror::Error;
use tracing::warn;

/// Represents a single block in the volatile blockchain state
///
/// Ideally under perfect network conditions and abscence of failures
/// this structure would be a linked list of blocks.
///
/// However due to network delays, partitions or malicious actors some
/// blocks might be missed by some validators and proposers might start
/// building new blocks off an older block, and that creates several
/// histories.
///
/// This data structure represents all known blockchain histories
/// since the last finalized block.
#[derive(Debug)]
struct TreeNode<D: BlockData> {
  value: VolatileBlock<D>,
  parent: Option<*mut TreeNode<D>>,
  children: Vec<Rc<RefCell<TreeNode<D>>>>,
}

impl<D: BlockData> TreeNode<D> {
  pub fn new(block: VolatileBlock<D>) -> Self {
    Self {
      value: block,
      parent: None,
      children: vec![],
    }
  }

  /// Returns a reference to a block with a given hash
  /// in the current subtree, or None if no such block is found.
  ///
  /// SAFETY: This struct and its methods are internal to this module
  /// and the node pointed to by the returned poineter is never reclaimed
  /// while reading the value retuned.
  unsafe fn _get(
    &self,
    hash: &Multihash,
  ) -> Result<Option<*const Self>, std::io::Error> {
    if self.value.block.hash()? == *hash {
      Ok(Some(self as *const Self))
    } else {
      for child in self.children.iter() {
        let child = child.borrow();
        if let Some(b) = child._get(hash)? {
          return Ok(Some(b));
        }
      }
      Ok(None)
    }
  }

  /// Returns a mutable reference to a block with a given hash
  /// in the current subtree, or None if no such block is found.
  /// SAFETY: This struct and its methods are internal to this module
  /// and the node pointed to by the returned poineter is never reclaimed
  /// while reading the value retuned.
  pub unsafe fn get_mut(
    &mut self,
    hash: &Multihash,
  ) -> Result<Option<*mut Self>, std::io::Error> {
    if self.value.block.hash()? == *hash {
      Ok(Some(self as *mut Self))
    } else {
      for child in self.children.iter_mut() {
        let mut child = child.borrow_mut();
        if let Some(b) = child.get_mut(hash)? {
          return Ok(Some(b));
        }
      }
      Ok(None)
    }
  }

  /// Returns the block that is currently considered the
  /// head of the fork subtree.
  ///
  /// The selection of this block uses the Greedy Heaviest
  /// Observed Subtree algorithm (GHOST), and it basically
  /// means that returns the last block from the subtree
  /// that has accumulated the largest amount of votes
  /// so far or highest slot number if there is a draw.
  pub fn head(&self) -> *const block::Produced<D> {
    if self.children.is_empty() {
      return &self.value.block; // leaf block
    }

    let mut max_votes = 0;
    let mut top_subtree = self
      .children
      .first()
      .expect("is_empty would have returned earlier");
    for subtree in &self.children {
      match subtree.borrow().value.votes.cmp(&max_votes) {
        Ordering::Less => { /* nothing, we have a better tree */ }
        Ordering::Equal => {
          // if two blocks have the same number of votes, select the one with
          // the greater height.
          if subtree.borrow().value.block.height()
            > top_subtree.borrow().value.block.height()
          {
            top_subtree = subtree;
          }
        }
        Ordering::Greater => {
          max_votes = subtree.borrow().value.votes;
          top_subtree = subtree;
        }
      }
    }

    // recursively keep finding the top subtree
    // until we get to a leaf block, then return it
    top_subtree.borrow().head()
  }

  /// Adds an immediate child to this forktree node.
  pub fn add_child(
    &mut self,
    block: VolatileBlock<D>,
  ) -> Result<(), std::io::Error> {
    assert!(block.block.parent()? == self.value.block.hash()?);

    let blockhash = block.block.hash()?;
    for child in self.children.iter() {
      if child.borrow().value.block.hash()? == blockhash {
        // block already a child of this block
        return Ok(());
      }
    }

    // set parent link to ourself
    let block = Rc::new(RefCell::new(TreeNode {
      value: block,
      parent: Some(self as *mut Self),
      children: vec![],
    }));

    self.children.push(block);

    Ok(())
  }

  /// Applies votes to a block, and all its ancestors until the
  /// last finalized block that is used as the justification for
  /// this vote.
  pub fn add_votes(&mut self, votes: u64, voter: Pubkey) {
    // apply those votes to the current block, but don't duplicate
    // validator votes on the same block.
    if self.value.voters.insert(voter.clone()) {
      self.value.votes += votes;
    }

    // also apply those votes to all the parent votes
    // until the justification point.
    let mut current = self;
    while let Some(ancestor) = current.parent {
      let ancestor = unsafe { &mut *ancestor as &mut Self };
      if ancestor.value.voters.insert(voter.clone()) {
        ancestor.value.votes += votes;
      }
      current = ancestor;
    }
  }
}

/// A block that is still not finalized and its votes
/// are still being counted.
///
/// Those blocks are not guaranteed to never be
/// discarded by the blockchain yet.
#[derive(Debug)]
struct VolatileBlock<D: BlockData> {
  block: block::Produced<D>,
  votes: u64,
  voters: HashSet<Pubkey>,
}

/// State of the unfinalized part of the chain.
/// Blocks in this state can be overriden using
/// fork choice rules and voting.
#[derive(Debug)]
pub struct VolatileState<D: BlockData> {
  root: Multihash,
  forrest: Vec<Rc<RefCell<TreeNode<D>>>>,
}

impl<D: BlockData> VolatileState<D> {
  pub fn new(root: Multihash) -> Self {
    Self {
      root,
      forrest: vec![],
    }
  }

  pub fn head(&self) -> Option<block::Produced<D>> {
    if self.forrest.is_empty() {
      return None;
    }

    let mut max_votes = 0;
    let mut top_tree = self
      .forrest
      .first()
      .expect("is_empty would have returned earlier");

    // find the subtree that has accumulated the highes number
    // of votes in the fork forrest.
    for tree in &self.forrest {
      if tree.borrow().value.votes > max_votes {
        max_votes = tree.borrow().value.votes;
        top_tree = tree;
      }
    }

    // then from that tree get the most voted on
    // child block or the one with the most recent
    // slot number of there is a draw in votes.
    Some(unsafe { &*top_tree.borrow().head() }.clone())
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
    let voter = block.signature.0.clone();
    let block = VolatileBlock {
      block,
      votes: stake, // block proposition is counted as a vote on the block
      voters: [voter].into_iter().collect(),
    };

    let parent = block.block.parent()?;

    // if we're the first block in the volatile state
    if self.forrest.is_empty() {
      if parent == self.root {
        self
          .forrest
          .push(Rc::new(RefCell::new(TreeNode::new(block))));
        Ok(())
      } else {
        warn!(
          "rejecting block. cannot find parent {}",
          bs58::encode(parent.to_bytes()).into_string()
        );
        Err(VolatileStateError::ParentBlockNotFound)
      }
    } else {
      // we already have some unfinalized blocks
      for root in self.forrest.iter_mut() {
        if let Some(b) = unsafe { root.borrow_mut().get_mut(&parent)? } {
          let b = unsafe { &mut *b as &mut TreeNode<D> };
          b.add_child(block)?;
          return Ok(());
        }
      }
      Err(VolatileStateError::ParentBlockNotFound)
    }
  }

  /// Adds a vote for a given target block in the history.
  ///
  /// The justification must be the last finalized block,
  /// and the target block must be one of its descendants.
  pub fn vote(
    &mut self,
    vote: Vote,
    stake: u64,
  ) -> Result<(), VolatileStateError> {
    if vote.justification != self.root {
      warn!("Not justified by the last finalized block: {vote:?}");
      return Err(VolatileStateError::InvalidJustification);
    }

    for root in self.forrest.iter_mut() {
      if let Some(b) = unsafe { root.borrow_mut().get_mut(&vote.target)? } {
        let b = unsafe { &mut *b as &mut TreeNode<D> };
        b.add_votes(stake, vote.validator);
        return Ok(());
      }
    }

    Err(VolatileStateError::VoteTargetNotFound)
  }
}

#[derive(Debug, Error)]
pub enum VolatileStateError {
  #[error("Block's parent hash is invalid: {0}")]
  InvalidParentBockHash(#[from] std::io::Error),

  #[error("Parent block not found")]
  ParentBlockNotFound,

  #[error("Invalid justification for vote")]
  InvalidJustification,

  #[error("The block hash being voted on is not found")]
  VoteTargetNotFound,
}
