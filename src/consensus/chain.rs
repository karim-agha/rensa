use super::{
  block::{self, Block, BlockData},
  validator::Validator,
  volatile::VolatileState,
  vote::Vote,
};
use crate::{primitives::Pubkey, vm::Finalized};
use dashmap::DashMap;
use std::ops::Deref;
use tracing::{info, warn};

/// Represents the state of the consensus protocol
pub struct Chain<'g, 'f, D: BlockData> {
  /// The very first block in the chain.
  ///
  /// This comes from a configuration file, is always considered
  /// as finalized and has special fields not present in produced blocks
  /// that configure the behaviour of the chain.
  genesis: &'g block::Genesis<D>,

  /// This is a dynamic collection of all known validators along
  /// with the amount of tokens they are staking (and their voting power).
  stakes: DashMap<Pubkey, u64>,

  /// This is the last block that was finalized and we are
  /// guaranteed that it will never be reverted. The runtime
  /// and the validator cares only about the state of the system
  /// at the last finalized block. Archiving historical blocks
  /// can be delegated to an external interface for explorers
  /// and other use cases if an archiver is specified.
  finalized: &'f Finalized<'f, D>,

  /// This tree represents all chains (forks) that were created
  /// since the last finalized block. None of those blocks are
  /// guaranteed to be finalized and each fork operates on a different
  /// view of the global state specific to the transactions executed
  /// within its path from the last finalized block.
  ///
  /// Those blocks are voted on by validators, once the finalization
  /// requirements are met, they get finalized.
  volatile: VolatileState<'f, D>,
}

impl<'g, 'f, D: BlockData> Chain<'g, 'f, D> {
  pub fn new(
    genesis: &'g block::Genesis<D>,
    finalized: &'f Finalized<'f, D>,
  ) -> Self {
    Self {
      genesis,
      finalized,
      volatile: VolatileState::new(finalized),
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
  pub fn finalized(&self) -> &Finalized<'f, D> {
    self.finalized
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
      None => *self.finalized.deref() as _,
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

impl<'g, 'f, D: BlockData> Chain<'g, 'f, D> {
  /// Called whenever a new block is received on the p2p layer.
  ///
  /// This method will validate signatures on the block and attempt
  /// to insert it into the volatile state of the chain.
  pub fn include(&mut self, block: block::Produced<D>) {
    if let Some(stake) = self.stakes.get(&block.signature.0) {
      if block.verify_signature() {
        if block.hash().is_ok() && block.parent().is_ok() {
          info!("ingesting block {block}",);
          self.volatile.include(block, *stake);
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

    self.volatile.vote(vote, stake);
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
  use std::{collections::HashMap, marker::PhantomData, time::Duration};

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
      hasher: multihash::Code::Sha3_256,
      slot_interval: Duration::from_secs(2),
      state: HashMap::new(),
      builtins: vec![],
      validators: vec![Validator {
        pubkey: keypair.public(),
        stake: 200000,
      }],
      _marker: PhantomData,
    };

    let finalized = Finalized {
      underlying: &genesis,
      state: FinalizedState,
    };

    let mut chain = Chain::new(&genesis, &finalized);
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
      genesis_time: Utc::now(),
      hasher: multihash::Code::Sha3_256,
      slot_interval: Duration::from_secs(2),
      state: HashMap::new(),
      builtins: vec![],
      validators: vec![Validator {
        pubkey: keypair.public(),
        stake: 200000,
      }],
      _marker: PhantomData,
    };

    let finalized = Finalized {
      underlying: &genesis,
      state: FinalizedState,
    };

    let mut chain = Chain::new(&genesis, &finalized);

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
