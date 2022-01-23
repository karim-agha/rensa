use super::{
  block::{self, Block, BlockData},
  validator::Validator,
  volatile::VolatileState,
  vote::Vote,
};
use crate::keys::Pubkey;
use dashmap::DashMap;
use multihash::Multihash;
use tracing::{info, warn};

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
  pub fn head(&self) -> Option<block::Produced<D>> {
    // no volatile state, either all blocks are finalized
    // or we are still at genesis block.
    match self.volatile.head() {
      Some(head) => Some(head),
      None => self.finalized.last().cloned(),
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
    keys::Keypair,
  };
  use chrono::Utc;
  use ed25519_dalek::{PublicKey, SecretKey};
  use std::time::Duration;

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

    assert!(chain.head().is_none());

    let hash = block.hash().unwrap();

    chain.include(block);
    assert!(chain.head().is_some());
    assert_eq!(hash, chain.head().unwrap().hash().unwrap());

    let block2 = block::Produced::new(
      &keypair,
      2,
      chain.head().unwrap().hash().unwrap(),
      "three".to_string(),
      vec![],
    )
    .unwrap();

    let hash2 = block2.hash().unwrap();
    chain.include(block2);
    assert_eq!(hash2, chain.head().unwrap().hash().unwrap());
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

    // no we should have only genesis
    assert!(chain.head().is_none());

    // block should be the head
    chain.include(block);
    assert!(chain.head().is_some());
    assert_eq!(hash, chain.head().unwrap().hash().unwrap());

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
    assert_eq!(hash, chain.head().unwrap().hash().unwrap());

    // include the missing parent, now the chain should match
    // it with the orphan and block3 shold be the new head
    chain.include(block2);
    assert_eq!(hash3, chain.head().unwrap().hash().unwrap());
  }
}
