use super::{
  block::{self, Block, BlockData},
  vote::Vote,
};
use crate::primitives::{Pubkey, ToBase58String};
use multihash::Multihash;
use std::{
  cell::RefCell,
  cmp::Ordering,
  collections::{hash_map::Entry, HashMap, HashSet},
  rc::Rc,
};
use tracing::{debug, info, warn};

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
  pub unsafe fn get_mut(&mut self, hash: &Multihash) -> Option<*mut Self> {
    if self.value.block.hash().expect("previously veriefied") == *hash {
      Some(self as *mut Self)
    } else {
      for child in self.children.iter_mut() {
        let mut child = child.borrow_mut();
        if let Some(b) = child.get_mut(hash) {
          return Some(b);
        }
      }
      None
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
  pub fn add_child(&mut self, block: VolatileBlock<D>) {
    assert!(block.block.parent().unwrap() == self.value.block.hash().unwrap());

    let blockhash = block.block.hash().unwrap();
    for child in self.children.iter() {
      if child.borrow().value.block.hash().unwrap() == blockhash {
        return;
      }
    }

    // set parent link to ourself
    let block = Rc::new(RefCell::new(TreeNode {
      value: block,
      parent: Some(self as *mut Self),
      children: vec![],
    }));

    self.children.push(block);
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
  /// Hash of the last justified block.
  /// Only blocks descending from this root are
  /// subject to the fork choice rules.
  root: Multihash,

  /// Blocks that were received but their parent
  /// block was not included in the forrest (yet).
  ///
  /// We keep those blocs in this dictionary, where
  /// the key is the hash of their parent.
  ///
  /// If such parent arrives, then those orphas are
  /// removed from this collection and attached as
  /// children of that parent.
  ///
  /// Otherwise, incoming blocks that have a parent inside
  /// the orphans collection are attached to the orphan tree.
  orphans: HashMap<Multihash, Vec<VolatileBlock<D>>>,

  /// A list of blockchain that have the justified block
  /// as their parent.
  forrest: Vec<Rc<RefCell<TreeNode<D>>>>,
}

impl<D: BlockData> VolatileState<D> {
  pub fn new(root: Multihash) -> Self {
    Self {
      root,
      orphans: HashMap::new(),
      forrest: vec![],
    }
  }

  pub fn head(&self) -> Option<&block::Produced<D>> {
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
    Some(unsafe { &*top_tree.borrow().head() })
  }

  /// Includes a newly received block into the volatile state
  ///
  /// The block might end up as part of one of the block trees
  /// in the fork tree or as an orphan if its parent is not found.
  pub fn include(&mut self, block: block::Produced<D>, stake: u64) {
    let voter = block.signature.0.clone();
    let block = VolatileBlock {
      block,
      votes: stake, // block proposition is counted as a vote on the block
      voters: [voter].into_iter().collect(),
    };

    let hash = block.block.hash().unwrap();
    if let Some(block) = self.append_or_return(block) {
      // the block was not accepted because its parent
      // was not found. store as orphan.
      // Maybe later we will have a block that turns
      // out to be its parent but is delayed due to
      // network or other reasons.
      self.add_orphan(block);
    } else {
      info!("Block {} included successfully.", hash.to_b58());
      // check if the successfully included block has any
      // orphan blocks that are its descendants
      self.match_orphans(hash);
    }
  }

  /// Tries to inserts a block into the current unfinalized fork tree.
  ///
  /// Returns None if block was successfully appended and it was inserted
  /// as a descendant of an existing block. Otherwise this method will move
  /// back the the volatile block object to the caller.
  ///
  /// The VolatileBlock type is not [`Clone`] or [`Copy`] intentionally,
  /// because there can exist only one valid instance of this type at any
  /// time and this is inforced by the type system.
  fn append_or_return(
    &mut self,
    block: VolatileBlock<D>,
  ) -> Option<VolatileBlock<D>> {
    let parent_hash = block.block.parent().expect("already verified");

    // is this block building right off the last finalized block?
    if parent_hash == self.root {
      let node = Rc::new(RefCell::new(TreeNode::new(block)));
      self.forrest.push(node);
      return None;
    }

    // try to append this block to one of the trees in the volatile
    // state by looking up its parent in every try, then inserting
    // it as a child of the found parent.
    for root in self.forrest.iter_mut() {
      if let Some(b) = unsafe { root.borrow_mut().get_mut(&parent_hash) } {
        let b = unsafe { &mut *b as &mut TreeNode<D> };
        b.add_child(block);
        return None;
      }
    }

    Some(block)
  }

  /// Adds a vote for a given target block in the history.
  ///
  /// The justification must be the last finalized block,
  /// and the target block must be one of its descendants.
  pub fn vote(&mut self, vote: Vote, stake: u64) {
    if vote.justification != self.root {
      warn!("Not justified by the last finalized block: {vote:?}");
      return;
    }

    for root in self.forrest.iter_mut() {
      if let Some(b) = unsafe { root.borrow_mut().get_mut(&vote.target) } {
        let b = unsafe { &mut *b as &mut TreeNode<D> };
        b.add_votes(stake, vote.validator);
        return;
      }
    }
  }

  /// Orphan blocks are blocks that were received by the network
  /// gossip but we don't have their parent block, so we can't
  /// attach them to any current block in the volatile state.
  ///
  /// This happens more often with shorter block times (under 2 sec)
  /// as later blocks might arrive before earlier blocks.
  ///
  /// We store those blocks in a special collection, indexed
  /// by their parent block. Whenever a block is correctly
  /// appended to the chain, we also look for any orphans that
  /// could be its children and append them to the chain.
  fn add_orphan(&mut self, block: VolatileBlock<D>) {
    let parent = block.block.parent;
    warn!(
      "parent block {} for {} not found (or has not arrived yet)",
      parent.to_b58(),
      block.block
    );
    match self.orphans.entry(parent) {
      Entry::Occupied(mut orphans) => {
        orphans.get_mut().push(block);
      }
      Entry::Vacant(v) => {
        v.insert(vec![block]);
      }
    };
  }

  /// For a given block hash, checks if we are aware of any
  /// orphan blocks that found its parent and if so, inserts
  /// them recursively into the forktree.
  fn match_orphans(&mut self, parent_hash: Multihash) {
    if let Some(orphans) = self.orphans.remove(&parent_hash) {
      debug!(
        "found {} orphan(s) of block {}",
        orphans.len(),
        parent_hash.to_b58()
      );
      for orphan in orphans {
        self.include(orphan.block, orphan.votes);
      }
    }
  }
}
