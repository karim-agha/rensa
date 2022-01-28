use super::{
  block::{self, Block, BlockData},
  validator::Validator,
  volatile::VolatileState,
};
use crate::{primitives::Pubkey, vm::Finalized};
use futures::{Stream, StreamExt};
use multihash::Multihash;
use std::{
  collections::{HashSet, VecDeque},
  pin::Pin,
  task::{Context, Poll},
};
use tracing::{info, warn};

#[derive(Debug)]
pub enum ChainEvent<D: BlockData> {
  Vote {
    target: Multihash,
    justification: Multihash,
  },
  BlockIncluded(block::Produced<D>),
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
  validators: HashSet<Pubkey>,

  /// This is the last block that was finalized and we are
  /// guaranteed that it will never be reverted. The runtime
  /// and the validator cares only about the state of the system
  /// at the last finalized block. Archiving historical blocks
  /// can be delegated to an external interface for explorers
  /// and other use cases if an archiver is specified.
  //finalized: &Finalized<'f, D>,

  /// This tree represents all chains (forks) that were created
  /// since the last finalized block. None of those blocks are
  /// guaranteed to be finalized and each fork operates on a different
  /// view of the global state specific to the transactions executed
  /// within its path from the last finalized block.
  ///
  /// Those blocks are voted on by validators, once the finalization
  /// requirements are met, they get finalized.
  volatile: VolatileState<D>,

  /// Events emitted by this chain instance
  events: VecDeque<ChainEvent<D>>,
}

impl<'g, D: BlockData> Chain<'g, D> {
  pub fn new(genesis: &'g block::Genesis<D>, finalized: Finalized<D>) -> Self {
    Self {
      genesis,
      volatile: VolatileState::new(finalized, &genesis.validators),
      events: VecDeque::new(),
      validators: genesis
        .validators
        .iter()
        .map(|v| v.pubkey.clone())
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
    &self.volatile.root
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
    // no volatile state, either all blocks are finalized
    // or we are still at genesis block.
    match self.volatile.head() {
      Some(head) => head,
      None => self.volatile.root.as_ref(),
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

  /// Given a block/slot height it returns
  /// the epoch number it belongs to
  fn epoch(&self, height: u64) -> u64 {
    height / self.genesis.epoch_slots
  }
}

impl<'g, 'f, D: BlockData> Chain<'g, D> {
  /// Called whenever a new block is received on the p2p layer.
  ///
  /// This method will validate signatures on the block and attempt
  /// to insert it into the volatile state of the chain.
  pub fn include(&mut self, block: block::Produced<D>) {
    if self.validators.contains(&block.signature.0) {
      if block.verify_signature() {
        if block.hash().is_ok() && block.parent().is_ok() {
          let bhash = block.hash().unwrap();
          info!(
            "ingesting block {block} in epoch {}",
            self.epoch(block.height())
          );
          self.volatile.include(block);
          // if the newly inserted block have successfully
          // replaced our head of the chain, then vote for it.
          if self.head().hash().unwrap() == bhash {
            self.commit_and_vote(bhash, self.finalized().hash().unwrap());
          }
        } else {
          warn!("rejecting block {block}. Unreadable hashes");
        }
      } else {
        warn!("signature verification failed for block {block}.");
      }
    } else {
      warn!(
        "Rejecting block {block} from non-staking proposer {}",
        block.signature.0
      );
    }
  }

  /// This is also called when the current validator is voting,
  /// it includes its own vote. Returns true if the vote was
  /// accepted and it should be propagated to the rest of the
  /// network, otherwise false.
  ///
  /// Two different methods are used for own votes vs foreign
  /// votes, so that validators won't violate the rules of consensus
  /// and commit to the fork branches they have voted on and avoid
  /// voting on conflicting branches.
  fn commit_and_vote(&mut self, target: Multihash, justification: Multihash) {
    // todo: commit to this branch and never vote
    // on a conflicting branch. That's a hard slashing condition.
    self.events.push_back(ChainEvent::Vote {
      target,
      justification,
    });
  }
}

impl<D: BlockData> Unpin for Chain<'_, D> {}
impl<D: BlockData> Stream for Chain<'_, D> {
  type Item = ChainEvent<D>;

  fn poll_next(
    mut self: Pin<&mut Self>,
    cx: &mut Context<'_>,
  ) -> Poll<Option<Self::Item>> {
    if let Some(event) = self.events.pop_back() {
      return Poll::Ready(Some(event));
    }
    if let Poll::Ready(Some(event)) = self.volatile.poll_next_unpin(cx) {
      return Poll::Ready(Some(event));
    }

    Poll::Pending
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
    primitives::Keypair,
    vm::{Finalized, FinalizedState, Transaction},
  };
  use chrono::Utc;
  use ed25519_dalek::{PublicKey, SecretKey};
  use std::{
    collections::BTreeMap, marker::PhantomData, rc::Rc, time::Duration,
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
      underlying: Rc::new(genesis.clone()),
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
      underlying: Rc::new(genesis.clone()),
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
