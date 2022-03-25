//! Blockchain State
//!
//! In general when blocks are produced by validators they go through three
//! phases:
//!
//! 1. Processed: When a block is first produced and included in the forktree it
//! is in    this phase until it gets at least 2/3 of all stake of votes. Those
//! blocks are fairly    volatile and should not be relied on for anything
//! serious, except maybe arbitrage bots    and MEV.
//!
//! 2. Confirmed: Once 2/3 of the stake votes on a block or any of its
//! descendants, it will    be in the confirmed state and that means that it is
//! very likely that this block will    soon be finalized.
//!
//! 3. Finalized: A block gets finalized when it is confirmed and in the
//! canonical chain for    two consecutive epochs. Finalized blocks are never
//! reverted and they can be used as    a confirmation for high-stakes
//! operations, such as withdrawals from exchanges, or payment    confirmations.
//!
//! The volatile state is a tree of chains that gets formed as validators rotate
//! and produce new blocks. In an ideal world with perfect network conditions
//! and no delays, this should be a linked list. However when a validator build
//! a block before receiving the latest block from the last producer, it sets
//! the parent block to the last received block and that's how forks are formed.
//! Forks may be also a result of a validator attempting to censor another
//! validator's blocks.
//!
//! Voting is the process of commiting to a branch of the forktree as the
//! canonical branch by other validators in the system. The forkchoice rules are
//! based on the Greedy Heaviest Obvserved SubTree (GHOST) algorithm in Casper.

use {
  super::{
    block::{self, Block, BlockData},
    forktree::{TreeNode, VolatileBlock},
    orphans::Orphans,
    validator::Validator,
    vote::Vote,
    Genesis,
    Produced,
  },
  crate::{
    primitives::{Pubkey, ToBase58String},
    vm::{self, Executed, Finalized, MachineError, Overlayed, State},
  },
  futures::Stream,
  multihash::Multihash,
  std::{
    cmp::Ordering,
    collections::{hash_map::Entry, HashMap, HashSet, VecDeque},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
  },
  tracing::{debug, warn},
};

#[derive(Debug)]
pub enum ChainEvent<D: BlockData> {
  /// Indicates that the consensus algorithm on the current
  /// validator casted a vote for a block that it has included.
  Vote {
    target: Multihash,
    justification: Multihash,
  },

  /// Indicates that a block in the forktree didn't make it into
  /// the confirmed branch and will not be processed further.
  ///
  /// This is a signal that the votes and transactions on the
  /// discarded block should be reincluded in a future block
  /// so they make it into a future confirmed branch.
  BlockDiscarded(Produced<D>),

  /// Indicates that there are blocks that are missing their
  /// parent block for a long time. This asks other validators
  /// that may have the missing block to replay it.
  BlockMissing(Multihash),

  /// Indicates that a block was successfully verified and included
  /// in the forktree but has not yet received more than 2/3rds vote.
  BlockIncluded(Executed<D>),

  /// Indicates that the block has already received more than 2/3rds
  /// of the voting stake and is in the canonical chain.
  BlockConfirmed { block: Executed<D>, votes: u64 },

  /// Indicates that a block has reached a point where it will never
  /// be reverted in any case and its state is final for the entire
  /// chain across all participating validators.
  BlockFinalized { block: Executed<D>, votes: u64 },
}

/// Represents the state of the consensus protocol
pub struct Chain<'g, D: BlockData> {
  /// The very first block in the chain.
  ///
  /// This comes from a configuration file, is always considered
  /// as finalized and has special fields not present in produced blocks
  /// that configure the behaviour of the chain.
  genesis: &'g Genesis<D>,

  /// This is a dynamic collection of all known validators along
  /// with the amount of tokens they are staking (and their voting power).
  stakes: HashMap<Pubkey, u64>,

  /// This is the last block that was finalized and we are
  /// guaranteed that it will never be reverted. The runtime
  /// and the validator cares only about the state of the system
  /// at the last finalized block. Archiving historical blocks
  /// can be delegated to an external interface for explorers
  /// and other use cases if an archiver is specified.
  finalized: Finalized<'g, D>,

  /// This forrest represents all chains (forks) that were created
  /// since the last finalized block. None of those blocks are
  /// guaranteed to be finalized and each fork operates on a different
  /// view of the global state specific to the transactions executed
  /// within its path from the last finalized block.
  ///
  /// Those blocks are voted on by validators, once the finalization
  /// requirements are met, they get finalized.
  forktrees: Vec<TreeNode<D>>,

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
  ///
  /// The value stored by this collection is the list of blocks
  /// that are waiting for the parent to arrive, and the time when
  /// this parent appeared on the list of missing parents.
  ///
  /// This time is used to trigger a block repley in case the p2p
  /// protocol didn't succeed in recovering a lost gossip through
  /// its mechanisms.
  orphans: Orphans<D>,

  /// Votes casted by this node.
  /// This collection is used to track the voting history
  /// of the current node and to ensure that it never votes
  /// for two conflicting branches of the history.
  ///
  /// The is a key-value mapping of epoch# -> (justification, target)
  ownvotes: HashMap<u64, (Multihash, Multihash)>,

  /// Events emitted by this chain instance
  events: VecDeque<ChainEvent<D>>,

  /// a list of all recently finalized blocks groupped
  /// by their epoch. The system stores N epochs that
  /// are valid justifications of epoch as defined in the
  /// genesis maxJustificationAge.
  finalized_history: HashMap<u64, HashSet<Multihash>>,

  /// The virtual machine that executes transactions
  /// contained within a block
  virtual_machine: &'g vm::Machine,
}

impl<'g, D: BlockData> Chain<'g, D> {
  pub fn new(
    genesis: &'g Genesis<D>,
    machine: &'g vm::Machine,
    finalized: Finalized<'g, D>,
  ) -> Self {
    let epoch_duration = genesis.slot_interval * genesis.epoch_blocks as u32;
    Self {
      genesis,
      finalized,
      forktrees: vec![],
      orphans: Orphans::new(epoch_duration),
      ownvotes: HashMap::new(),
      events: VecDeque::new(),
      finalized_history: HashMap::new(),
      stakes: genesis
        .validators
        .iter()
        .map(|v| (v.pubkey, v.stake))
        .collect(),
      virtual_machine: machine,
    }
  }

  /// Executes a closure with the block that is currently
  /// considered the head of the chain and the accumulated
  /// state global state at that block in its fork path.
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
  /// This method returns the last finalized block if the
  /// volatile history is empty ingested or produced so far.
  pub fn with_head<R>(
    &self,
    mut op: impl FnMut(&dyn State, &dyn Block<D>) -> R,
  ) -> R {
    let mut heads: Vec<_> = self.forktrees.iter().map(|f| f.head()).collect();

    // get the most voted on subtree and if there is a draw, get
    // the longest chain
    heads.sort_by(|a, b| match a.value.votes.cmp(&b.value.votes) {
      Ordering::Equal => a.depth().cmp(&b.depth()),
      o => o,
    });

    // if no volatile state, either all blocks
    // are finalized or we are still at genesis block.
    let head_block = match heads.last() {
      Some(head) => &*head.value.block,
      None => self.finalized.as_ref(),
    };

    let base_state = self.finalized.state();

    match heads.last() {
      Some(head) => op(&Overlayed::new(base_state, &head.state()), head_block),
      None => op(base_state, head_block),
    }
  }

  /// Represents the current set of validators that are
  /// taking part in the consensus. For now, this value
  /// is static and based on what is defined in genesis.
  ///
  /// In the next iteration validators will be able to
  /// join and leave the blockchain.
  fn _validators(&self) -> impl Iterator<Item = &Validator> {
    self
      .genesis
      .validators
      .iter()
      .filter(|v| v.stake < self.genesis.limits.minimum_stake)
  }

  /// The sum of all staked tokens that are taking part in
  /// the consensus.
  pub fn total_stake(&self) -> u64 {
    self
      .stakes
      .iter()
      .filter(|(_, s)| **s >= self.genesis.limits.minimum_stake)
      .fold(0, |a, (_, s)| a + s)
  }

  /// The minimum voted stake that constitutes a 2/3 majority
  pub fn minimum_majority_stake(&self) -> u64 {
    (self.total_stake() as f64 * 0.67f64).ceil() as u64
  }

  /// Given a block/slot height it returns
  /// the epoch number it belongs to
  fn epoch(&self, block: &dyn Block<D>) -> u64 {
    block.height() / self.genesis.epoch_blocks
  }
}

impl<'g, 'f, D: BlockData> Chain<'g, D> {
  /// checks if a block has received at least 2/3 of stake votes
  fn confirmed(&self, block: &VolatileBlock<D>) -> bool {
    block.votes >= self.minimum_majority_stake()
  }

  /// Locates a node in the fork trees that has a block with a given hash.
  fn get_block_node(&self, hash: &Multihash) -> Option<&dyn Block<D>> {
    for root in &self.forktrees {
      if let Some(node) = root.get(hash) {
        return Some(node.value.block.underlying.as_ref());
      }
    }

    if self.finalized.hash().unwrap() == *hash {
      return Some(&**self.finalized);
    }

    None
  }

  /// Checks if a given hash is a hash of a finalized block within
  /// N (specified in genesis) blocks, or if it is hash of genesis
  /// itself if not enough epochs have been finalized.
  fn in_finalized_history(&self, hash: &Multihash) -> bool {
    &self.finalized.hash().unwrap() == hash
      || self.finalized_history.iter().any(|(_, e)| e.contains(hash))
      || (self.finalized_history.len()
        < self.genesis.limits.max_justification_age as usize
        && hash == &self.genesis.hash().unwrap())
  }

  /// Adds a vote for a given target block in the history.
  ///
  /// The justification must be the last finalized block,
  /// and the target block must be one of its descendants.
  fn injest_vote(&mut self, vote: &Vote) {
    if let Some(stake) = self.stakes.get(&vote.validator) {
      if *stake < self.genesis.limits.minimum_stake {
        debug!(
          "Rejecting vote from {} because it has not enough stake",
          vote.validator
        );
        return;
      }

      if let Err(err) = vote.verify_signature() {
        warn!("Signature verification failed for vote {vote:?}: {err:?}");
        return;
      }

      for root in self.forktrees.iter_mut() {
        if let Some(target) = root.get_mut(&vote.target) {
          let target = unsafe { &mut *target as &mut TreeNode<D> };

          // verify that the justification is a known finalized block
          // or an ancestor of the voted on block
          if !self.in_finalized_history(&vote.justification)
            && !target.is_descendant_of(&vote.justification)
          {
            warn!(
              "Vote justification not found: {}",
              vote.justification.to_b58()
            );
            return;
          }

          // find out which block are unconfirmed prior to the vote
          let unconfirmed: Vec<_> = target
            .path() // from all ancestors
            .filter(|b| !self.confirmed(&b.value)) // select unconfirmed yet
            .map(|b| &b.value as *const _) // work around the borrow checker by storing a ptr
            .collect();

          // apply votes to the target and all its ancestors
          target.add_votes(*stake, vote.validator);

          // find out which blocks got confirmed after counting the vote
          // and signal their confirmation by emitting an event
          self.events.append(
            &mut unconfirmed
              .iter()
              .map(|u| unsafe { &**u as &_ }) // borrow checker workaround, back to ref from ptr
              .filter(|b| self.confirmed(b)) // only the ones that become confirmed
              .map(|b| ChainEvent::BlockConfirmed { // generate a new event
                block: b.block.clone(),
                votes: b.votes,
              })
              .into_iter()
              .collect(),
          );
          return;
        }
      }

      // the vote target is not available yet.
      // store those votes and when the target
      // block arrives, count them.
      debug!(
        "Vote {:?} target not available yet, storing for later.",
        vote
      );
      self.orphans.add_vote(vote.clone());
    } else {
      warn!("Ignoring vote from unknown validator {}", vote.validator);
    }
  }

  /// Count and apply all votes in a block
  fn count_votes(&mut self, votes: &[Vote]) {
    for vote in votes {
      self.injest_vote(vote);
    }
  }

  /// Attempts to insert the block into the blocktree
  /// under its parent block. If the parent is found
  /// then the block is consumed and None is returned.
  ///
  /// If the block is successfully inserted under a parent
  /// its votes will also be counted and later used to determine
  /// the head of the chain.
  ///
  /// Otherwise the block is returned back to its caller.
  fn try_include(
    &mut self,
    block: Produced<D>,
  ) -> Result<Result<(), Produced<D>>, MachineError> {
    if self.get_block_node(&block.hash().unwrap()).is_some() {
      // duplicate block, most likely replied for some other
      // validator after an explicit replay request.
      debug!("Ignoring duplicate block {block}.");
      return Ok(Ok(()));
    }

    let mut emit_event = |block: &Executed<D>| {
      self
        .events
        .push_front(ChainEvent::BlockIncluded(block.clone()));
    };

    if block.parent == self.finalized.hash().unwrap() {
      if block.height != self.finalized.height() + 1 {
        return Err(MachineError::InvalidBlockHeight);
      }

      // this block is a root in the forktree so it operates
      // on the finalized state directly.
      let block = VolatileBlock::new(Executed::new(
        self.finalized.state(),
        Arc::new(block),
        self.virtual_machine,
      )?);

      self.forktrees.push(TreeNode::new(block));
      emit_event(&self.forktrees.last().unwrap().value.block);
      return Ok(Ok(()));
    } else {
      for tree in self.forktrees.iter_mut() {
        if let Some(parent) = tree.get_mut(&block.parent) {
          let parent = unsafe { &mut *parent as &mut TreeNode<D> };

          if block.height != parent.value.height() + 1 {
            return Err(MachineError::InvalidBlockHeight);
          }

          // this block operates on a state that is generated by ancestor
          // blocks that have not been finalized yet, so the state object
          // that the VM receives for executing this block is a union of
          // all parent blocks state and the finalized state with priority
          // given to most recent blocks.
          parent.add_child(VolatileBlock::new(Executed::new(
            &Overlayed::new(self.finalized.state(), &parent.state()),
            Arc::new(block),
            self.virtual_machine,
          )?));
          emit_event(&parent.children.last().unwrap().value.block);
          return Ok(Ok(()));
        }
      }
    }

    Ok(Err(block))
  }

  /// Called whenever a new block is received on the p2p layer.
  ///
  /// This method will validate signatures on the block and attempt
  /// to insert it into the volatile state of the chain.
  pub fn include(&mut self, block: block::Produced<D>) {
    if !self.stakes.contains_key(&block.signature.0) {
      warn!(
        "Rejecting block {block} from non-staking proposer {}",
        block.signature.0
      );
      return;
    }

    if *self.stakes.get(&block.signature.0).unwrap()
      < self.genesis.limits.minimum_stake
    {
      warn!(
        "Rejecting block {block} from {} because it has not enough stake.",
        block.signature.0
      );
      return;
    }

    if block.hash().is_err() || block.parent().is_err() {
      warn!("rejecting block {block}. Unreadable hashes");
      return;
    }

    let finalized_height = self.finalized.height();
    if block.height() <= finalized_height {
      // block irrelevant, most likely coming from a replay
      // request that is not interesting for this validator
      debug!(
        "Block {block} is too old. Current finalized height is {}",
        finalized_height
      );
      return;
    }

    if !block.verify_signature() {
      warn!("signature verification failed for block {block}.");
      return;
    }

    let bhash = block.hash().unwrap();
    debug!("ingesting block {block} in epoch {}", self.epoch(&block));

    // try inserting the new block into the chain by looking
    // for its parent block and adding it as a child.
    match self.try_include(block) {
      Ok(result) => match result {
        // the block was included and consumed
        Ok(()) => {
          // if the newly inserted block have successfully
          // replaced our head of the chain, then vote for it.
          if bhash == self.with_head(|_, block| block.hash().unwrap()) {
            self.commit_and_vote(bhash);
          }

          // check if any of the previously orphaned blocks
          // is a child of the newly inserted block
          if let Some(orphans) = self.orphans.consume_blocks(&bhash) {
            // now consume the entire orphan tree that was pending
            // on the block just inserted.
            for orphan in orphans {
              self.include(orphan);
            }
          }
        }
        // the block was not matched with a parent and returned
        // to the caller, store it as an orphan.
        Err(block) => {
          // no known block is a valid parent of this block.
          // store it in the orphans collection and try matching
          // it later with its parent as new blocks arrive.
          self.orphans.add_block(block)
        }
      },
      Err(vmerror) => {
        warn!("Block {} rejected: {:?}", bhash.to_b58(), vmerror);
      }
    }
  }

  /// Given a block hash this will try to generate a vote for it
  /// using CBC Casper voting rules. Each vote has a justification
  /// that is a parent of the target block that has already received
  /// 2/3 votes.
  /// While trying to generate a vote it makes sure that it does
  /// not violate the two voting faults of CBC Casper:
  ///   1. No conflicting votes;
  ///   2. No surround vote;
  fn commit_and_vote(&mut self, target: Multihash) {
    for root in &self.forktrees {
      if let Some(target) = root.get(&target) {
        let epoch = self.epoch(&*target.value);

        // The justification is the last finalized block.
        let justification_hash = self.finalized.hash().unwrap();
        let target_hash = target.value.hash().unwrap();

        // if we have already voted in this epoch, make sure that
        // we are not violating any voting rules.
        if let Some((j, t)) = self.ownvotes.get(&epoch) {
          // 1. no surround vote, never use a justification
          // that is an ancestor of a previous justification.
          if j != &justification_hash && !self.in_finalized_history(j) {
            return; // this will create a slashable surround vote.
          }

          // make sure that we are voting only on descendants of our
          // previous votes, and not on conflicting forks.
          if !target.is_descendant_of(t) {
            return; // otherwise it will create a slashable conflicting vote.
          }
        }

        // save our vote
        self
          .ownvotes
          .insert(epoch, (justification_hash, target_hash));

        debug!(
          "voting for block {} with justification {}",
          target_hash.to_b58(),
          justification_hash.to_b58(),
        );
        self.events.push_front(ChainEvent::Vote {
          target: target_hash,
          justification: justification_hash,
        });
        return;
      }
    }
  }

  /// looks a root in the fork trees that is eligible
  /// for finalization and returns its index.
  fn find_finalizable_root(&self) -> Option<usize> {
    for (i, root) in self.forktrees.iter().enumerate() {
      if root.value.votes >= self.minimum_majority_stake() {
        let head = root.head();

        // we need to find two consecutive epochs that have
        // 2/3 of the votig stake, then we can finalize the
        // second ancestor and all its parents.

        // first rewind to the beginning of the epoch of the current head
        let head_epoch_start = head.epoch_start(self.genesis.epoch_blocks);

        if let Some(first_checkpoint) = head_epoch_start
          .path()
          .nth(1) // last block in previous epoch
          .map(|c| c.epoch_start(self.genesis.epoch_blocks))
        {
          // check if the preceeding epoch is confirmed.
          if first_checkpoint.value.votes >= self.minimum_majority_stake() {
            // now check the second consecutive epoch checkpoint
            if let Some(second_checkpoint) = first_checkpoint
              .path()
              .nth(1) // last block in epoch N - 2
              .map(|c| c.epoch_start(self.genesis.epoch_blocks))
            {
              // the second consecutive checkpoint is confirmed
              // all ancestors of this block are considered final
              if second_checkpoint.value.votes >= self.minimum_majority_stake()
              {
                // move out the entire fork subtree,
                // it'll become the new finalized block,
                // and its children the forktree roots
                return Some(i);
              }
            }
          }
        }
      }
    }

    None
  }

  /// Given a root that was in the forktrees collection,
  /// this method will make the root the newest finalized
  /// state and its children the new top-level roots.
  ///
  /// Applies state accumulate by the root to the current
  /// finalized state.
  fn finalize_root(&mut self, subtree: TreeNode<D>) {
    let newroot_hash = subtree.value.hash().unwrap();
    // apply root's state diff and set it as the new finalized block
    self.finalized.apply(subtree.value.block);

    // this is the list of trees in the forktree that didn't make
    // it and will be removed permanently from the consensus tree.
    //
    // This list is collected to mark those blocks as "Discarded",
    // so that votes and transactions included in them can be reincluded
    // in a future block.
    let discarded = self
      .forktrees
      .iter()
      .filter(|tree| tree.value.hash().unwrap() != newroot_hash);

    // traverse all discarded blocks
    fn visit<D: BlockData>(
      node: &TreeNode<D>,
      op: &mut impl FnMut(&TreeNode<D>),
    ) {
      op(node);
      for child in node.children.iter() {
        visit(child, op);
      }
    }

    // signal an event with a discarded block
    for root in discarded {
      visit(root, &mut |node| {
        self.events.push_front(ChainEvent::BlockDiscarded(
          node.value.block.underlying.as_ref().clone(),
        ));
      });
    }

    self.forktrees = subtree
      .children
      .into_iter()
      .map(|mut c| {
        c.parent = None; // becomes new root in forktree
        c
      })
      .collect();
  }

  /// A block is finalized if two consecutive epochs get 2/3 majority votes.
  ///
  /// The intuition behind it is:
  /// The first 2/3 vote acknowledges that: "I know that everyone
  /// thinks this is the correct fork", the second consecutive vote
  /// acknoledges that: "I know that everyone knows that everyone thinks
  /// this is the correct fork".
  ///
  /// We go over all the roots of the fork tree and if we find a
  /// root that has 67% of stake and its descendant one epoch later
  /// also has 67% of the votes, then we discard all other roots,
  /// move the root to a finalized state, and its children become
  /// the new roots.
  ///
  /// Returns true if a root was finalized, otherwise false.
  fn try_finalize_roots(&mut self) -> bool {
    // check if any of the forktree roots is eligible for
    // being finalized.
    if let Some(index) = self.find_finalizable_root() {
      // found one, remove it from the tree and
      // finalize it, then make its children the
      // new forktrees roots.
      let subtree = self.forktrees.remove(index);

      // clones is for the output event
      let votes = subtree.value.votes;
      let block = subtree.value.block.clone();

      self.finalize_root(subtree);

      // keep this collection size bounded,
      // finalized votes are irrelevant for new votes.
      self.ownvotes.remove(&self.epoch(&*block));

      // signal to external listeners that a block was finalized
      self
        .events
        .push_front(ChainEvent::BlockFinalized { block, votes });
      return true;
    }
    false
  }
}

impl<'g, D: BlockData> Chain<'g, D> {
  /// Invoked whenever a block is successfully included in the forktree
  fn post_block_included(&mut self, block: &Produced<D>) {
    self.count_votes(block.votes());
    if let Some(votes) = self.orphans.consume_votes(&block.hash().unwrap()) {
      for vote in votes {
        debug!("counting late vote {:?}", vote);
        self.injest_vote(&vote);
      }
    }
    while self.try_finalize_roots() {}

    // unstuck the consensus if it is missing blocks
    // for too long.
    for missing in self.orphans.missing_blocks(self.finalized.height()) {
      self.events.push_front(ChainEvent::BlockMissing(missing));
    }
  }

  /// Invoked whenever a block reaches finality
  fn post_block_finalized(&mut self, block: &Produced<D>) {
    // keep a sliding window of recently finalized blocks from
    // up to N specified in genesis epochs.
    // only those blocks are valid justifications of a vote.
    let epoch = self.epoch(self.finalized.as_ref());
    match self.finalized_history.entry(epoch) {
      Entry::Occupied(mut e) => {
        e.get_mut().insert(block.hash().unwrap());
      }
      Entry::Vacant(e) => {
        e.insert([block.hash().unwrap()].into_iter().collect());
      }
    };

    // clear old epochs
    let window = self.genesis.limits.max_justification_age;
    self
      .finalized_history
      .retain(|e, _| e >= &epoch.saturating_sub(window));
  }
}

impl<'g, D: BlockData> Chain<'g, D> {
  /// Attempts to retreive a non-finalized block that is still
  /// going through the consensus algorithm.
  pub fn get(&self, hash: &Multihash) -> Option<&Executed<D>> {
    for root in &self.forktrees {
      if let Some(node) = root.get(hash) {
        return Some(&node.value.block);
      }
    }
    None
  }
}

impl<D: BlockData> Unpin for Chain<'_, D> {}
impl<D: BlockData> Stream for Chain<'_, D> {
  type Item = ChainEvent<D>;

  fn poll_next(
    mut self: Pin<&mut Self>,
    _: &mut Context<'_>,
  ) -> Poll<Option<Self::Item>> {
    if let Some(event) = self.events.pop_back() {
      if let ChainEvent::BlockIncluded(ref block) = event {
        self.post_block_included(block);
      }

      if let ChainEvent::BlockFinalized { ref block, .. } = event {
        self.post_block_finalized(block);
      }

      return Poll::Ready(Some(event));
    }

    Poll::Pending
  }
}

#[cfg(test)]
mod test {
  use {
    super::Chain,
    crate::{
      consensus::{
        block::{self, Block},
        genesis::Limits,
        validator::Validator,
        Genesis,
      },
      primitives::Keypair,
      storage::PersistentState,
      vm::{self, Executable, Finalized, Transaction},
    },
    chrono::Utc,
    ed25519_dalek::{PublicKey, SecretKey},
    std::{
      collections::BTreeMap,
      marker::PhantomData,
      sync::Arc,
      time::Duration,
    },
  };

  #[test]
  fn append_block_smoke() {
    let secret = SecretKey::from_bytes(&[
      157, 97, 177, 157, 239, 253, 90, 96, 186, 132, 74, 244, 146, 236, 44,
      196, 68, 73, 197, 105, 123, 50, 105, 25, 112, 59, 172, 3, 28, 174, 127,
      96,
    ])
    .unwrap();

    let public: PublicKey = (&secret).into();
    let keypair: Keypair = ed25519_dalek::Keypair { secret, public }.into();

    let genesis = Genesis::<Vec<Transaction>> {
      chain_id: "1".to_owned(),
      epoch_blocks: 32,
      genesis_time: Utc::now(),
      slot_interval: Duration::from_secs(2),
      state: BTreeMap::new(),
      builtins: vec![],
      limits: Limits {
        max_block_size: 100_000,
        max_justification_age: 100,
        minimum_stake: 100,
        max_log_size: 512,
        max_logs_count: 32,
        max_account_size: 65536,
        max_input_accounts: 32,
        max_block_transactions: 2000,
        max_contract_size: 614400,
        max_transaction_params_size: 2048,
      },
      system_coin: "RensaToken1xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
        .parse()
        .unwrap(),
      validators: vec![Validator {
        pubkey: keypair.public(),
        stake: 200000,
      }],
      _marker: PhantomData,
    };

    let mut randomdir = std::env::temp_dir();
    randomdir.push("append_block_smoke");
    let storage = PersistentState::new(&genesis, randomdir.clone()).unwrap();
    let finalized = Finalized::new(Arc::new(genesis.clone()), &storage);
    let vm = vm::Machine::new(&genesis).unwrap();
    let mut chain = Chain::new(&genesis, &vm, finalized);

    let (first_hash, statehash) = chain.with_head(|s, b| {
      // blocks have no txs, so the statehash won't change across
      // blocks, but it needs to be a valid hash otherwise the block
      // gets rejected and not appended to the chain.
      (b.hash().unwrap(), *vec![].execute(&vm, s).unwrap().hash())
    });

    let block = block::Produced::new(
      &keypair,
      1,
      genesis.hash().unwrap(),
      vec![],
      statehash,
      vec![],
    )
    .unwrap();

    assert_eq!(first_hash, genesis.hash().unwrap());

    let hash = block.hash().unwrap();

    chain.include(block);
    chain.with_head(|_, block| {
      assert_eq!(hash, block.hash().unwrap());
    });

    let block2 = block::Produced::new(
      &keypair,
      2,
      chain.with_head(|_, b| b.hash().unwrap()),
      vec![],
      statehash,
      vec![],
    )
    .unwrap();

    let hash2 = block2.hash().unwrap();
    chain.include(block2);
    assert_eq!(hash2, chain.with_head(|_, b| b.hash().unwrap()));

    drop(storage);

    std::fs::remove_dir_all(randomdir).unwrap();
  }

  #[test]
  fn append_blocks_out_of_order() {
    let secret = SecretKey::from_bytes(&[
      157, 97, 177, 157, 239, 253, 90, 96, 186, 132, 74, 244, 146, 236, 44,
      196, 68, 73, 197, 105, 123, 50, 105, 25, 112, 59, 172, 3, 28, 174, 127,
      96,
    ])
    .unwrap();

    let public: PublicKey = (&secret).into();
    let keypair: Keypair = ed25519_dalek::Keypair { secret, public }.into();

    let genesis = Genesis {
      chain_id: "1".to_owned(),
      epoch_blocks: 32,
      genesis_time: Utc::now(),
      slot_interval: Duration::from_secs(2),
      state: BTreeMap::new(),
      builtins: vec![],
      limits: Limits {
        max_block_size: 100_000,
        max_justification_age: 100,
        minimum_stake: 100,
        max_log_size: 512,
        max_logs_count: 32,
        max_account_size: 65536,
        max_input_accounts: 32,
        max_block_transactions: 2000,
        max_contract_size: 614400,
        max_transaction_params_size: 2048,
      },
      system_coin: "RensaToken1xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
        .parse()
        .unwrap(),
      validators: vec![Validator {
        pubkey: keypair.public(),
        stake: 200000,
      }],
      _marker: PhantomData,
    };

    let mut randomdir = std::env::temp_dir();
    randomdir.push("append_blocks_out_of_order");
    let storage = PersistentState::new(&genesis, randomdir.clone()).unwrap();
    let finalized = Finalized::new(Arc::new(genesis.clone()), &storage);

    let vm = vm::Machine::new(&genesis).unwrap();
    let mut chain = Chain::new(&genesis, &vm, finalized);

    let (first_hash, statehash) = chain.with_head(|s, b| {
      // blocks have no txs, so the statehash won't change across
      // blocks, but it needs to be a valid hash otherwise the block
      // gets rejected and not appended to the chain.
      (b.hash().unwrap(), *vec![].execute(&vm, s).unwrap().hash())
    });

    let block = block::Produced::new(
      &keypair,
      1,
      genesis.hash().unwrap(),
      "two".to_string(),
      statehash,
      vec![],
    )
    .unwrap();
    let hash = block.hash().unwrap();

    // no we should have only genesis
    assert_eq!(first_hash, genesis.hash().unwrap());

    // block should be the head
    chain.include(block);
    chain.with_head(|_, block| {
      assert_eq!(hash, block.hash().unwrap());
    });

    let block2 = block::Produced::new(
      &keypair,
      2,
      hash,
      "three".to_string(),
      statehash,
      vec![],
    )
    .unwrap();
    let hash2 = block2.hash().unwrap();

    let block3 = block::Produced::new(
      &keypair,
      3,
      hash2,
      "four".to_string(),
      statehash,
      vec![],
    )
    .unwrap();
    let hash3 = block3.hash().unwrap();

    // out of order insertion, the head should not change
    // after block3, instead it should be stored as an orphan
    chain.include(block3);
    chain.with_head(|_, block| {
      assert_eq!(hash, block.hash().unwrap());
    });

    // include the missing parent, now the chain should match
    // it with the orphan and block3 shold be the new head
    chain.include(block2);
    chain.with_head(|_, block| {
      assert_eq!(hash3, block.hash().unwrap());
    });

    drop(storage);

    std::fs::remove_dir_all(randomdir).unwrap();
  }
}
