use super::{
  block::{self, Block, BlockData},
  chain::ChainEvent,
  validator::Validator,
  vote::Vote,
};
use crate::{
  primitives::{Pubkey, ToBase58String},
  vm::{Finalized, FinalizedState},
};
use futures::Stream;
use multihash::Multihash;
use std::{
  cell::RefCell,
  cmp::Ordering,
  collections::{hash_map::Entry, HashMap, HashSet, VecDeque},
  mem::take,
  pin::Pin,
  rc::Rc,
  task::{Context, Poll},
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
  pub fn head(&self) -> *const TreeNode<D> {
    if self.children.is_empty() {
      return self; // leaf block
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
          // if two blocks have the same number of votes,
          // select the one with the longest chain.
          if subtree.borrow().depth() > top_subtree.borrow().depth() {
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

    // insert the block into this fork subtree as a leaf
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

  /// The distance of this node from the root of the tree.
  /// This is used in determining the longest current chain.
  pub fn depth(&self) -> usize {
    let mut depth = 0;
    let mut current = self;
    while let Some(ancestor) = current.parent {
      current = unsafe { &mut *ancestor as &mut Self };
      depth += 1;
    }
    depth
  }
}

/// A block that is still not finalized and its votes
/// are still being counted.
///
/// Those blocks are not guaranteed to never be
/// discarded by the blockchain yet.
#[derive(Debug, Clone)]
struct VolatileBlock<D: BlockData> {
  block: block::Produced<D>,
  votes: u64,
  voters: HashSet<Pubkey>,
}

impl<D: BlockData> VolatileBlock<D> {
  pub fn new(block: block::Produced<D>) -> Self {
    Self {
      block,
      votes: 0,
      voters: HashSet::new(),
    }
  }
}

/// State of the unfinalized part of the chain.
/// Blocks in this state can be overriden using
/// fork choice rules and voting.
#[derive(Debug)]
pub struct VolatileState<D: BlockData> {
  /// Hash of the last finalized block.
  ///
  /// Only blocks descending from this root are
  /// subject to the fork choice rules and could
  /// be reverted.
  pub(super) root: Finalized<D>,

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

  /// This is a dynamic collection of all known validators along
  /// with the amount of tokens they are staking (and their voting power).
  stakes: HashMap<Pubkey, u64>,

  /// A list of blockchain that have the justified block
  /// as their parent.
  forrest: Vec<Rc<RefCell<TreeNode<D>>>>,

  /// Events emitted by this chain instance
  events: VecDeque<ChainEvent<D>>,
}

impl<D: BlockData> VolatileState<D> {
  pub fn new(root: Finalized<D>, validators: &[Validator]) -> Self {
    Self {
      root,
      stakes: validators
        .iter()
        .map(|v| (v.pubkey.clone(), v.stake))
        .collect(),
      orphans: HashMap::new(),
      events: VecDeque::new(),
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
      match tree.borrow().value.votes.cmp(&max_votes) {
        Ordering::Less => { /* nothing, we have a better tree */ }
        Ordering::Equal => {
          // if two trees have the same number of votes, select the one with
          // the longest chain.
          let top_tree_head =
            unsafe { &*top_tree.borrow().head() as &TreeNode<D> };
          let current_tree_head =
            unsafe { &*tree.borrow().head() as &TreeNode<D> };

          if current_tree_head.depth() > top_tree_head.depth() {
            top_tree = tree;
          }
        }
        Ordering::Greater => {
          max_votes = tree.borrow().value.votes;
          top_tree = tree;
        }
      }
      if tree.borrow().value.votes > max_votes {
        max_votes = tree.borrow().value.votes;
        top_tree = tree;
      }
    }

    // then from that tree get the most voted on
    // child block or the one with the most recent
    // slot number of there is a draw in votes.
    Some(unsafe {
      &(*top_tree.borrow().head()).value.block as &block::Produced<D>
    })
  }

  /// Includes a newly received block into the volatile state
  ///
  /// The block might end up as part of one of the block trees
  /// in the fork tree or as an orphan if its parent is not found.
  pub fn include(&mut self, block: block::Produced<D>) {
    if block.height() < self.root.height() {
      warn!(
        "Rejecting block {block} because it is older than the latest finalized block {}",
        self.root.height()
      );
      return;
    }

    let bclone = block.clone();
    let block = VolatileBlock::new(block);

    if let Some(block) = self.append_or_return(block) {
      // the block was not accepted because its parent
      // was not found. store as orphan.
      // Maybe later we will have a block that turns
      // out to be its parent but is delayed due to
      // network or other reasons.
      self.add_orphan(block);
    } else {
      // apply all votes in this block to consensus forks
      self.count_votes(&bclone.votes);

      // inform any external observer about the fact that
      // a block was included in the chain.
      let hash = bclone.hash().unwrap();
      self.events.push_back(ChainEvent::BlockIncluded(bclone));

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
    if parent_hash == self.root.hash().unwrap() {
      for child in &self.forrest {
        if child.borrow().value.block.hash().unwrap()
          == block.block.hash().unwrap()
        {
          // already there, it is a duplicate
          warn!(
            "rejecting a duplicate block {}.",
            block.block.hash().unwrap().to_b58()
          );
          return None;
        }
      }
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

  fn _total_stake(&self) -> u64 {
    self.stakes.iter().fold(0, |a, (_, s)| a + s)
  }

  /// The minimum voted stake that constitutes a 2/3 majority
  fn _minimum_majority_stake(&self) -> u64 {
    (self._total_stake() as f64 * 0.67f64).ceil() as u64
  }

  /// Count and apply all votes in a block
  fn count_votes(&mut self, votes: &[Vote]) {
    for vote in votes {
      self.vote(vote);
    }

    //self.try_finalize_roots();
  }

  /// Adds a vote for a given target block in the history.
  ///
  /// The justification must be the last finalized block,
  /// and the target block must be one of its descendants.
  fn vote(&mut self, vote: &Vote) {
    if vote.justification != self.root.hash().unwrap() {
      warn!(
        "Not justified by the last finalized block {}: {vote:?}",
        self.root.hash().unwrap().to_b58()
      );
      return;
    }

    if let Some(stake) = self.stakes.get(&vote.validator) {
      for root in self.forrest.iter_mut() {
        if let Some(b) = unsafe { root.borrow_mut().get_mut(&vote.target) } {
          let b = unsafe { &mut *b as &mut TreeNode<D> };
          b.add_votes(*stake, vote.validator.clone());
        }
      }
    }
  }

  /// a block is finalized if two consecutive blocks in a row
  /// get 2/3 majority votes. The intuition behind it is:
  /// The first 2/3 vote acknowledges that: "I know that everyone
  /// thinks this is the correct fork", the second consecutive vote
  /// acknoledges that: "I know that everyone knows that everyone thinks
  /// this is the correct fork".
  ///
  /// We go over all the roots of the fork tree and if we find a
  /// root that has 67% of stake with a direct descendant that
  /// also has 67% of stake votes then the root is finalized,
  /// and other children of the current root are discarded.
  fn _try_finalize_roots(&mut self) {
    let total = self._total_stake();
    let majority = self._minimum_majority_stake();

    let final_root = || {
      for root in self.forrest.iter() {
        if root.borrow().value.votes > majority {
          for child in &root.borrow().children {
            if child.borrow().value.votes > majority {
              return Some(Rc::clone(root));
            }
          }
        }
      }
      None
    };

    if let Some(root) = final_root() {
      // discard all branches except the finalized
      // and move the fork tree one level deeper towards
      // the finalized block.
      self.forrest = take(&mut root.borrow_mut().children);

      // todo apply executed state
      self.root = Finalized {
        underlying: Rc::new(root.borrow().value.block.clone()),
        state: FinalizedState,
      };

      info!(
        "Block {} is finalized with {:.2}% stake",
        root.borrow().value.block.hash().unwrap().to_b58(),
        (root.borrow().value.votes as f64 / total as f64) * 100f64,
      );
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
        self.include(orphan.block);
      }
    }
  }
}

impl<D: BlockData> Unpin for VolatileState<D> {}
impl<D: BlockData> Stream for VolatileState<D> {
  type Item = ChainEvent<D>;

  fn poll_next(
    mut self: Pin<&mut Self>,
    _: &mut Context<'_>,
  ) -> Poll<Option<Self::Item>> {
    if let Some(event) = self.events.pop_back() {
      return Poll::Ready(Some(event));
    }
    Poll::Pending
  }
}
