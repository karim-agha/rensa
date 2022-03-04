use {
  super::{BlockData, Produced, Vote},
  crate::{consensus::Block, primitives::ToBase58String},
  multihash::Multihash,
  std::{
    collections::{hash_map::Entry, HashMap, VecDeque},
    time::{Duration, Instant},
  },
  tracing::warn,
};

struct Node<D: BlockData> {
  at: Instant,
  block: Produced<D>,
  children: HashMap<Multihash, Node<D>>,
}

impl<D: BlockData> Node<D> {
  pub fn new(block: Produced<D>) -> Self {
    Self {
      block,
      at: Instant::now(),
      children: HashMap::new(),
    }
  }

  /// height of the root of the orphan tree
  pub fn height(&self) -> u64 {
    self.block.height
  }

  /// height of the deepest child of this root
  pub fn max_height(&self) -> u64 {
    let mut h = self.height();

    fn traverse<D: BlockData>(n: &Node<D>, mut sofar: u64) -> u64 {
      for child in n.children.values() {
        sofar = traverse(child, sofar).max(sofar);
      }
      sofar.max(n.height())
    }

    for child in self.children.values() {
      h = traverse(child, h).max(h);
    }
    h
  }

  /// Attempts to insert a block into an orphan tree.
  /// Consumes the block on success and returnes it on failure.
  pub fn insert(&mut self, new: Produced<D>) -> Result<(), Produced<D>> {
    if new.height <= self.height() {
      return Err(new); // must be a descendant
    }

    let newhash = new.hash().unwrap();
    let selfhash = self.block.hash().unwrap();

    // direct descendant case:
    if new.parent == selfhash {
      // only if not already present:
      if let Entry::Vacant(e) = self.children.entry(newhash) {
        e.insert(Node::new(new));
      }
      return Ok(());
    }

    // maybe deeper descendant:
    let mut new = Some(new);
    for (_, node) in self.children.iter_mut() {
      match node.insert(new.unwrap()) {
        Ok(()) => return Ok(()),
        Err(block) => {
          new = Some(block);
        }
      }
    }

    // does not belong to this orphan tree
    Err(new.take().unwrap())
  }

  /// how long this subtree has been an orphan
  pub fn since(&self) -> Duration {
    Instant::now().duration_since(self.at)
  }

  /// Sets the timestamp of this orphan tree to now.
  ///
  /// So that when checking for missing blocks, it
  /// will wait for another round of expirity.
  pub fn reset_timer(&mut self) {
    self.at = Instant::now();
  }

  /// converts this orphan tree into a flat list of
  /// blocks using breadth-first ordering, so the
  /// when included in that order in the chain they
  /// all end up finding their parent and become part
  /// of the chain.
  pub fn flatten(self) -> Vec<Produced<D>> {
    let mut output = vec![];
    let mut queue = VecDeque::new();
    queue.push_front(self);

    // order all nodes in BFT order
    while let Some(node) = queue.pop_back() {
      output.push(node.block);
      for (_, child) in node.children.into_iter() {
        queue.push_front(child);
      }
    }
    output
  }
}

/// Orphan blocks are blocks that were received by the network
/// gossip but we don't have their parent block, so we can't
/// attach them to any current block in the volatile state.
///
/// This happens more often with shorter block times (under 2 sec)
/// as later blocks might arrive before earlier blocks.
///
/// We store those blocks in this data structure, indexed
/// by their parent block. Whenever a block is correctly
/// appended to the chain, we also look for any orphans that
/// could be its children and append them to the chain.
pub struct Orphans<D: BlockData> {
  slot: Duration,
  blocks: HashMap<Multihash, Node<D>>,

  /// Votes that have not matched any target land in here.
  /// Once the target arrives, then those votes are counted.
  votes: HashMap<Multihash, Vec<Vote>>,
}

impl<D: BlockData> Orphans<D> {
  pub fn new(slot: Duration) -> Self {
    Self {
      votes: HashMap::new(),
      blocks: HashMap::new(),
      slot,
    }
  }

  pub fn add_vote(&mut self, vote: Vote) {
    match self.votes.entry(vote.target) {
      Entry::Occupied(mut v) => {
        v.get_mut().push(vote);
      }
      Entry::Vacant(v) => {
        v.insert(vec![vote]);
      }
    };
  }

  pub fn add_block(&mut self, block: Produced<D>) {
    let mut block = Some(block);

    // first try to insert it into one of
    // the existing orphan trees
    for root in self.blocks.values_mut() {
      match root.insert(block.unwrap()) {
        Ok(()) => return,
        Err(b) => block = Some(b),
      }
    }

    // didn't fit into any, create
    // new orphan tree rooted at this block
    let block = block.unwrap();
    let parent = block.parent;
    let block_string = format!("{block}");
    self.blocks.insert(block.parent, Node::new(block));

    warn!(
      "parent block {} for {} not found (or has not arrived yet)",
      parent.to_b58(),
      block_string
    );
  }

  pub fn consume_votes(&mut self, block: &Multihash) -> Option<Vec<Vote>> {
    self.votes.remove(block)
  }

  pub fn consume_blocks(
    &mut self,
    parent_hash: &Multihash,
  ) -> Option<Vec<Produced<D>>> {
    self.blocks.remove(parent_hash).map(Node::flatten)
  }

  /// If a block is missing for too long and orphans are waiting for
  /// too long for their parent, explicitly ask all peers to replay
  /// that specific block, otherwise consensus will be halted forever
  /// and loses its ability to confirm blocks because all new blocks
  /// end up being orphans.
  ///
  /// This is a method to mitigate that, of the orphans are more recent
  /// than the highest confirmed block, then request them to be replayed,
  /// otherwise, if they are older, then they are irrelevant anyway, discard.
  ///
  /// When a replay request is issued for a given block, the timer is reset
  /// and re-requested after that interval.
  ///
  /// At the moment blocks are considered missing if the were not received
  /// for longer then slots since its first reported.
  pub fn missing_blocks(
    &mut self,
    min_relevant_height: u64,
  ) -> impl Iterator<Item = Multihash> {
    let missing_threshold = self.slot * 2;

    let mut output = vec![];
    let mut irrelevant = vec![];
    for (hash, subtree) in self.blocks.iter_mut() {
      // if the orphans of a missing block belong to a height that is
      // older than the finalized state, prune them, as the are irrelevant
      // to consensus anymore.
      if subtree.max_height() <= min_relevant_height {
        irrelevant.push(*hash);
      } else if subtree.since() >= missing_threshold {
        subtree.reset_timer();
        output.push(*hash);
      }
    }

    // delete old irrelevant orphans of discarded branches that
    // did not make it to the canonical chain.
    for old in irrelevant {
      self.blocks.remove(&old);
    }

    output.into_iter()
  }
}
