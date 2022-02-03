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

use std::{
  cmp::Ordering,
  collections::{hash_map::Entry, HashMap, HashSet, VecDeque},
  pin::Pin,
  task::{Context, Poll},
};

use futures::Stream;
use multihash::Multihash;
use tracing::{debug, warn};

use super::{
  block::{self, Block, BlockData},
  forktree::{TreeNode, VolatileBlock},
  validator::Validator,
  vote::Vote,
};
use crate::{
  primitives::{Pubkey, ToBase58String},
  vm::{Finalized, FinalizedState, State},
};

#[derive(Debug)]
pub enum ChainEvent<D: BlockData> {
  Vote {
    target: Multihash,
    justification: Multihash,
  },
  BlockIncluded(block::Produced<D>),
  BlockConfirmed {
    block: block::Produced<D>,
    votes: u64,
  },
  BlockFinalized {
    block: block::Produced<D>,
    votes: u64,
  },
}

/// Represents the state of the consensus protocol
pub struct Chain<'g, D: BlockData> {
  /// The very first block in the chain.
  ///
  /// This comes from a configuration file, is always considered
  /// as finalized and has special fields not present in produced blocks
  /// that configure the behaviour of the chain.
  genesis: &'g block::Genesis<D>,

  /// This is a dynamic collection of all known validators along
  /// with the amount of tokens they are staking (and their voting power).
  stakes: HashMap<Pubkey, u64>,

  /// This is the last block that was finalized and we are
  /// guaranteed that it will never be reverted. The runtime
  /// and the validator cares only about the state of the system
  /// at the last finalized block. Archiving historical blocks
  /// can be delegated to an external interface for explorers
  /// and other use cases if an archiver is specified.
  finalized: Finalized<D>,

  /// This forrest represents all chains (forks) that were created
  /// since the last finalized block. None of those blocks are
  /// guaranteed to be finalized and each fork operates on a different
  /// view of the global state specific to the transactions executed
  /// within its path from the last finalized block.
  ///
  /// Those blocks are voted on by validators, once the finalization
  /// requirements are met, they get finalized.
  forktrees: Vec<Box<TreeNode<D>>>,

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

  /// Votes casted by this node.
  /// This collection is used to track the voting history
  /// of the current node and to ensure that it never votes
  /// for two conflicting branches of the history.
  ///
  /// The is a key-value mapping of epoch# -> (justification, target)
  ownvotes: HashMap<u64, (Multihash, Multihash)>,

  /// Events emitted by this chain instance
  events: VecDeque<ChainEvent<D>>,

  /// a list of all recently finalized blocks
  finalized_history: VecDeque<Multihash>,
}

impl<'g, D: BlockData> Chain<'g, D> {
  pub fn new(genesis: &'g block::Genesis<D>, finalized: Finalized<D>) -> Self {
    Self {
      genesis,
      finalized,
      forktrees: vec![],
      orphans: HashMap::new(),
      ownvotes: HashMap::new(),
      events: VecDeque::new(),
      finalized_history: VecDeque::new(),
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

  /// Returns the last finalized block in the chain.
  ///
  /// Blocks that reached finality will never be reverted under
  /// any circumstances.
  ///
  /// If no block has been finalized yet, then the genesis block
  /// hash is used as the last finalized block.
  ///
  /// This value is used as the justification when voting for new
  /// blocks, also the last finalized block is the root of the
  /// current fork tree.
  pub fn finalized(&self) -> &Finalized<D> {
    &self.finalized
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
  /// This method returns the last finalized block if the
  /// volatile history is empty ingested or produced so far.
  pub fn head(&self) -> &dyn Block<D> {
    let mut heads: Vec<_> = self.forktrees.iter().map(|f| f.head()).collect();

    // get the most voted on subtree and if there is a draw, get
    // the longest chain
    heads.sort_by(|a, b| match a.value.votes.cmp(&b.value.votes) {
      Ordering::Equal => a.depth().cmp(&b.depth()),
      o => o,
    });

    // if no volatile state, either all blocks
    // are finalized or we are still at genesis block.
    match heads.last() {
      Some(head) => &head.value.block,
      None => self.finalized.as_ref(),
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

  /// The sum of all staked tokens that are taking part in
  /// the consensus.
  pub fn total_stake(&self) -> u64 {
    self.stakes.iter().fold(0, |a, (_, s)| a + s)
  }

  /// The minimum voted stake that constitutes a 2/3 majority
  pub fn minimum_majority_stake(&self) -> u64 {
    (self.total_stake() as f64 * 0.67f64).ceil() as u64
  }

  /// Given a block/slot height it returns
  /// the epoch number it belongs to
  fn epoch(&self, height: u64) -> u64 {
    height / self.genesis.epoch_slots
  }
}

impl<'g, 'f, D: BlockData> Chain<'g, D> {
  /// checks if a block has received at least 2/3 of stake votes
  fn confirmed(&self, block: &VolatileBlock<D>) -> bool {
    block.votes >= self.minimum_majority_stake()
  }

  /// Adds a vote for a given target block in the history.
  ///
  /// The justification must be the last finalized block,
  /// and the target block must be one of its descendants.
  fn vote(&mut self, vote: &Vote) {
    if let Some(stake) = self.stakes.get(&vote.validator) {
      for root in self.forktrees.iter_mut() {
        if let Some(target) = root.get_mut(&vote.target) {
          let target = unsafe { &mut *target as &mut TreeNode<D> };

          // verify that the justification is a finalized block
          if vote.justification != self.finalized.hash().unwrap() {
            if !self.finalized_history.contains(&vote.justification) {
              warn!(
                "Vote justification not found: {}",
                vote.justification.to_b58()
              );
              return;
            }
          }

          let mut unconfirmed = vec![];
          if !self.confirmed(&target.value) {
            'confirmations: for ancestor in target.path() {
              if !self.confirmed(&ancestor.value) {
                unconfirmed.push(&ancestor.value as *const _);
              } else {
                break 'confirmations;
              }
            }
          }

          target.add_votes(*stake, vote.validator.clone());

          for ancestor in unconfirmed {
            let ancestor = unsafe { &*ancestor as &_ };
            if self.confirmed(ancestor) {
              self.events.push_back(ChainEvent::BlockConfirmed {
                block: ancestor.block.clone(),
                votes: ancestor.votes,
              })
            }
          }
          return;
        }
      }
      warn!("Vote target {} not found ", vote.target.to_b58());
    } else {
      warn!("Ignoring vote from unknown validator {}", vote.validator);
    }
  }

  /// Count and apply all votes in a block
  fn count_votes(&mut self, votes: &[Vote]) {
    for vote in votes {
      self.vote(vote);
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
    block: VolatileBlock<D>,
  ) -> Option<VolatileBlock<D>> {
    let mut emit_event = |block: &block::Produced<D>| {
      self
        .events
        .push_back(ChainEvent::BlockIncluded(block.clone()));
    };

    if block.parent == self.finalized.hash().unwrap() {
      self.forktrees.push(Box::new(TreeNode::new(block)));
      emit_event(&self.forktrees.last().unwrap().value.block);
      return None;
    } else {
      for tree in self.forktrees.iter_mut() {
        if let Some(parent) = tree.get_mut(&block.parent) {
          let parent = unsafe { &mut *parent as &mut TreeNode<D> };
          parent.add_child(block);
          emit_event(&parent.children.last().unwrap().value);
          return None;
        }
      }
    }

    Some(block)
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
        let ohash = orphan.hash().unwrap();

        // must succeed because the parent
        // was just inserted into the tree
        // before this round of orphan matching.
        assert!(self.try_include(orphan).is_none());

        // recursively for every newly included block
        // check if there are any orphans that found
        // its parent, and include them in the chain.
        self.match_orphans(ohash);
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

    if !block.verify_signature() {
      warn!("signature verification failed for block {block}.");
      return;
    }

    if block.hash().is_err() || block.parent().is_err() {
      warn!("rejecting block {block}. Unreadable hashes");
      return;
    }

    let bhash = block.hash().unwrap();
    debug!(
      "ingesting block {block} in epoch {}",
      self.epoch(block.height())
    );

    // wrap in a volatile block that can count votes
    let block = VolatileBlock::new(block);

    // try inserting the new block into the chain by looking
    // for its parent block and adding it as a child.
    match self.try_include(block) {
      // the block was included and consumed
      None => {
        // check if any of the previously orphaned blocks
        // is a child of the newly inserted block
        self.match_orphans(bhash);

        // if the newly inserted block have successfully
        // replaced our head of the chain, then vote for it.
        if self.head().hash().unwrap() == bhash {
          self.commit_and_vote(bhash);
        }
      }
      // the block was not matched with a parent and returned
      // to the caller, store it as an orphan.
      Some(block) => {
        // no known block is a valid parent of this block.
        // store it in the orphans collection and try matching
        // it later with its parent as new blocks arrive.
        self.add_orphan(block)
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
        let epoch = self.epoch(target.value.height());

        // The justification is the last finalized block.
        let justification_hash = self.finalized.hash().unwrap();
        let target_hash = target.value.hash().unwrap();

        // if we have already voted in this epoch, make sure that
        // we are not braking any voting rules.
        if let Some((j, t)) = self.ownvotes.get(&epoch) {
          // 1. no surround vote, never use a justification
          // that is an ancestor of a previous justification.
          if let Some(prev_just) = root.get(j) {
            if prev_just.is_descendant_of(&justification_hash) {
              return; // this will create a slashable surround vote.
            }
          }

          if !target.is_descendant_of(t) {
            return; // this will create a slashable conflicting vote.
          }
        }

        // save our vote
        self
          .ownvotes
          .insert(epoch, (justification_hash, target_hash));

        self.events.push_back(ChainEvent::Vote {
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
        let head_epoch_start = head.epoch_start(self.genesis.epoch_slots);

        if let Some(first_checkpoint) = head_epoch_start
          .path()
          .nth(1) // last block in previous epoch
          .map(|c| c.epoch_start(self.genesis.epoch_slots))
        {
          // check if the preceeding epoch is confirmed.
          if first_checkpoint.value.votes >= self.minimum_majority_stake() {
            // now check the second consecutive epoch checkpoint
            if let Some(second_checkpoint) = first_checkpoint
              .path()
              .nth(1) // last block in epoch N - 2
              .map(|c| c.epoch_start(self.genesis.epoch_slots))
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
    self.finalized = Finalized {
      underlying: Box::new(subtree.value.block),
      state: FinalizedState,
    };

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

      self.finalize_root(*subtree);

      // keep this collection size bounded,
      // finalized votes are irrelevant for new votes.
      self.ownvotes.remove(&self.epoch(block.height()));

      // signal to external listeners that a block was finalized
      self
        .events
        .push_back(ChainEvent::BlockFinalized { block, votes });
      return true;
    }
    false
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
        self.count_votes(block.votes());
        while self.try_finalize_roots() {}
      }

      if let ChainEvent::BlockFinalized { ref block, .. } = event {
        self.finalized_history.push_front(block.hash().unwrap());
        while self.finalized_history.len()
          > self.genesis.epoch_slots as usize * 4
        {
          self.finalized_history.pop_back();
        }
      }

      return Poll::Ready(Some(event));
    }

    Poll::Pending
  }
}

#[cfg(test)]
mod test {
  use std::{collections::BTreeMap, marker::PhantomData, time::Duration};

  use chrono::Utc;
  use ed25519_dalek::{PublicKey, SecretKey};

  use super::Chain;
  use crate::{
    consensus::{
      block::{self, Block, Genesis},
      validator::Validator,
    },
    primitives::Keypair,
    vm::{Finalized, FinalizedState, Transaction},
  };

  #[test]
  fn append_block_smoke() {
    let secret = SecretKey::from_bytes(&[
      157, 097, 177, 157, 239, 253, 090, 096, 186, 132, 074, 244, 146, 236,
      044, 196, 068, 073, 197, 105, 123, 050, 105, 025, 112, 059, 172, 003,
      028, 174, 127, 096,
    ])
    .unwrap();

    let public: PublicKey = (&secret).into();
    let keypair: Keypair = ed25519_dalek::Keypair { secret, public }.into();

    let genesis = Genesis::<Vec<Transaction>> {
      chain_id: "1".to_owned(),
      epoch_slots: 32,
      genesis_time: Utc::now(),
      max_block_size: 100_000,
      slot_interval: Duration::from_secs(2),
      state: BTreeMap::new(),
      builtins: vec![],
      validators: vec![Validator {
        pubkey: keypair.public(),
        stake: 200000,
      }],
      _marker: PhantomData,
    };

    let finalized = Finalized {
      underlying: Box::new(genesis.clone()),
      state: FinalizedState,
    };

    let mut chain = Chain::new(&genesis, finalized);
    let block = block::Produced::new(
      &keypair,
      1,
      genesis.hash().unwrap(),
      vec![],
      vec![],
    )
    .unwrap();

    assert_eq!(chain.head().hash().unwrap(), genesis.hash().unwrap());

    let hash = block.hash().unwrap();

    chain.include(block);
    assert_eq!(hash, chain.head().hash().unwrap());

    let block2 = block::Produced::new(
      &keypair,
      2,
      chain.head().hash().unwrap(),
      vec![],
      vec![],
    )
    .unwrap();

    let hash2 = block2.hash().unwrap();
    chain.include(block2);
    assert_eq!(hash2, chain.head().hash().unwrap());
  }

  #[test]
  fn append_blocks_out_of_order() {
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
      epoch_slots: 32,
      max_block_size: 100_000,
      genesis_time: Utc::now(),
      slot_interval: Duration::from_secs(2),
      state: BTreeMap::new(),
      builtins: vec![],
      validators: vec![Validator {
        pubkey: keypair.public(),
        stake: 200000,
      }],
      _marker: PhantomData,
    };

    let finalized = Finalized {
      underlying: Box::new(genesis.clone()),
      state: FinalizedState,
    };

    let mut chain = Chain::new(&genesis, finalized);

    let block = block::Produced::new(
      &keypair,
      1,
      genesis.hash().unwrap(),
      "two".to_string(),
      vec![],
    )
    .unwrap();
    let hash = block.hash().unwrap();

    // no we should have only genesis
    assert_eq!(chain.head().hash().unwrap(), genesis.hash().unwrap());

    // block should be the head
    chain.include(block);
    assert_eq!(hash, chain.head().hash().unwrap());

    let block2 =
      block::Produced::new(&keypair, 2, hash, "three".to_string(), vec![])
        .unwrap();
    let hash2 = block2.hash().unwrap();

    let block3 =
      block::Produced::new(&keypair, 3, hash2, "four".to_string(), vec![])
        .unwrap();
    let hash3 = block3.hash().unwrap();

    // out of order insertion, the head should not change
    // after block3, instead it should be stored as an orphan
    chain.include(block3);
    assert_eq!(hash, chain.head().hash().unwrap());

    // include the missing parent, now the chain should match
    // it with the orphan and block3 shold be the new head
    chain.include(block2);
    assert_eq!(hash3, chain.head().hash().unwrap());
  }
}
