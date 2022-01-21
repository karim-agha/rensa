use super::{
  block::{self, Block, BlockData},
  validator::Validator,
  vote::Vote,
};
use crate::keys::Pubkey;
use dashmap::DashMap;
use multihash::Multihash;
use std::{
  cmp::Ordering,
  collections::HashSet,
  rc::{Rc, Weak},
};
use thiserror::Error;
use tracing::{info, warn};

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
  parent: Option<Weak<TreeNode<D>>>,
  children: Vec<Rc<TreeNode<D>>>,
}

impl<D: BlockData> TreeNode<D> {
  pub fn new(block: VolatileBlock<D>) -> Self {
    Self {
      value: block,
      parent: None,
      children: vec![],
    }
  }

  /// Returns a mutable reference to a block with a given hash
  /// in the current subtree, or None if no such block is found.
  pub fn get(
    self: &Rc<Self>,
    hash: &Multihash,
  ) -> Result<Option<Rc<Self>>, std::io::Error> {
    if self.value.block.hash()? == *hash {
      Ok(Some(Rc::clone(self)))
    } else {
      for child in self.children.iter() {
        if let Some(ref b) = child.get(hash)? {
          return Ok(Some(Rc::clone(b)));
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
  pub fn head(&self) -> &block::Produced<D> {
    if self.children.is_empty() {
      return &self.value.block; // leaf block
    }

    let mut max_votes = 0;
    let mut top_subtree = self
      .children
      .first()
      .expect("is_empty would have returned earlier");
    for subtree in &self.children {
      match subtree.value.votes.cmp(&max_votes) {
        Ordering::Less => { /* nothing, we have a better tree */ }
        Ordering::Equal => {
          // if two blocks have the same number of votes, select the one with
          // the greater height.
          if subtree.value.block.height() > top_subtree.value.block.height() {
            top_subtree = subtree;
          }
        }
        Ordering::Greater => {
          max_votes = subtree.value.votes;
          top_subtree = subtree;
        }
      }
    }

    // recursively keep finding the top subtree
    // until we get to a leaf block, then return it
    top_subtree.head()
  }

  /// Adds an immediate child to this forktree node.
  pub fn add_child(
    mut self: Rc<Self>,
    block: VolatileBlock<D>,
  ) -> Result<(), std::io::Error> {
    assert!(block.block.parent()? == self.value.block.hash()?);

    let blockhash = block.block.hash()?;
    for child in self.children.iter() {
      if child.value.block.hash()? == blockhash {
        // block already a child of this block
        return Ok(());
      }
    }

    // set parent link and wrap in treenode
    let block = Rc::new(TreeNode {
      value: block,
      parent: Some(Rc::downgrade(&self)),
      children: vec![],
    });

    // store
    Rc::get_mut(&mut self).unwrap().children.push(block);

    Ok(())
  }

  /// Applies votes to a block, and all its ancestors until the
  /// last finalized block that is used as the justification for
  /// this vote.
  pub fn add_votes(mut self: Rc<Self>, votes: u64, voter: Pubkey) {
    // apply those votes to the current block, but don't duplicate
    // validator votes on the same block.
    let selfmut = Rc::get_mut(&mut self).unwrap();
    if selfmut.value.voters.insert(voter.clone()) {
      selfmut.value.votes += votes;
    }

    // also apply those votes to all the parent votes
    // until the justification point.
    let mut current = self;
    while let Some(ancestor) =
      Rc::get_mut(&mut current).unwrap().parent.as_mut()
    {
      let mut ancestor = ancestor.upgrade().unwrap();
      let mutancestor = Rc::get_mut(&mut ancestor).unwrap();
      if mutancestor.value.voters.insert(voter.clone()) {
        mutancestor.value.votes += votes;
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
struct VolatileState<D: BlockData> {
  root: Multihash,
  forrest: Vec<Rc<TreeNode<D>>>,
}

impl<D: BlockData> VolatileState<D> {
  pub fn new(root: Multihash) -> Self {
    Self {
      root,
      forrest: vec![],
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
        self.forrest.push(Rc::new(TreeNode::new(block)));
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
        info!("checking {root:?} in the forrest");
        if let Some(b) = root.get(&parent)? {
          info!("getting {b:?} from tree");
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
      if let Some(b) = root.get(&vote.target)? {
        b.add_votes(stake, vote.validator);
        return Ok(());
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
  volatile: VolatileState<D>,
}

impl<'g, D: BlockData> Chain<'g, D> {
  pub fn new(genesis: &'g block::Genesis<D>) -> Self {
    Self {
      genesis,
      finalized: vec![],
      volatile: VolatileState::new(genesis.hash().unwrap()),
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

  /// Returns the block that is currently considered the
  /// head of the chain.
  ///
  /// The selection of this block uses the Greedy Heaviest
  /// Observed Subtree algorithm (GHOST), and it basically
  /// means that returns the last block from the subtree
  /// that has accumulated the largest amount of votes
  /// so far.
  ///
  /// This method is called when a proposer needs to propose
  /// a new block and wants to know the parent block it needs
  /// to build on and use it as its parent.
  ///
  /// This method returns None when the volatile history
  /// is empty and no blocks were finalized so far, which
  /// means that we are still at the genesis block.
  pub fn head(&self) -> Option<&block::Produced<D>> {
    // no volatile state, either all blocks are finalized
    // or we are still at genesis block.
    if self.volatile.forrest.is_empty() {
      return self.finalized.last();
    }

    let mut max_votes = 0;
    let mut top_tree = self
      .volatile
      .forrest
      .first()
      .expect("is_empty would have returned earlier");

    // find the subtree that has accumulated the highes number
    // of votes in the fork forrest.
    for tree in &self.volatile.forrest {
      if tree.value.votes > max_votes {
        max_votes = tree.value.votes;
        top_tree = tree;
      }
    }

    // then from that tree get the most voted on
    // child block or the one with the most recent
    // slot number of there is a draw in votes.
    Some(top_tree.head())
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
  pub fn append(&mut self, block: block::Produced<D>) {
    if let Some(stake) = self.stakes.get(&block.signature.0) {
      if block.verify_signature() {
        info!("ingesting block {block:?}");
        if let Err(e) = self.volatile.append(block, *stake) {
          warn!("block rejected: {e}");
        }
      } else {
        warn!("rejecting block {block:?}: signature verification failed.");
      }
    } else {
      warn!(
        "Rejecting block from non-staking proposer {}",
        block.signature.0
      );
    }
  }

  /// Called whenever a new vote is received on the p2p layer.
  ///
  /// This method will validate signatures on the vote and attempt
  /// to insert it into the volatile state of the chain.
  pub fn vote(&mut self, vote: Vote) {
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

    if let Err(e) = self.volatile.vote(vote, stake) {
      warn!("vote rejected: {e}");
    }
  }
}

#[cfg(test)]
mod test {
  use super::Chain;
  use crate::{
    consensus::{
      block::{self, Block, Genesis},
      validator::Validator,
    },
    keys::Keypair,
  };
  use chrono::Utc;
  use ed25519_dalek::{PublicKey, SecretKey};
  use std::time::Duration;

  #[test]
  fn append_block() {
    let secret = SecretKey::from_bytes(&[
      157, 097, 177, 157, 239, 253, 090, 096, 186, 132, 074, 244, 146, 236,
      044, 196, 068, 073, 197, 105, 123, 050, 105, 025, 112, 059, 172, 003,
      028, 174, 127, 096,
    ])
    .unwrap();

    let public: PublicKey = (&secret).into();
    let keypair: Keypair = ed25519_dalek::Keypair { secret, public }.into();

    let genesis = Genesis {
      chain_id: "1".to_owned(),
      data: "test".to_owned(),
      epoch_slots: 32,
      genesis_time: Utc::now(),
      hasher: multihash::Code::Sha3_256,
      slot_interval: Duration::from_secs(2),
      validators: vec![Validator {
        pubkey: keypair.public(),
        stake: 200000,
      }],
    };

    let mut chain = Chain::new(&genesis);
    let block = block::Produced::new(
      &keypair,
      1,
      genesis.hash().unwrap(),
      "two".to_string(),
      vec![],
    )
    .unwrap();

    let hash = block.hash().unwrap();

    assert!(chain.head().is_none());

    chain.append(block);
    assert!(chain.head().is_some());
    assert_eq!(hash, chain.head().unwrap().hash().unwrap());
  }
}
