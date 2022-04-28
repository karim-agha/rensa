use {
  super::block::{self, Block, BlockData},
  crate::{
    primitives::{Account, Pubkey},
    vm::{Executed, State, StateError},
  },
  multihash::Multihash,
  std::{cmp::Ordering, collections::HashSet, ops::Deref},
};

/// A block that is still not finalized and its votes
/// are still being counted.
///
/// Those blocks are not guaranteed to never be
/// discarded by the blockchain yet until they are
/// voted on and finalized.
#[derive(Debug, Clone)]
pub struct VolatileBlock<D: BlockData> {
  pub block: Executed<D>,
  pub votes: u64,
  pub voters: HashSet<Pubkey>,
}

impl<D: BlockData> Deref for VolatileBlock<D> {
  type Target = block::Produced<D>;

  fn deref(&self) -> &Self::Target {
    &self.block
  }
}

impl<D: BlockData> VolatileBlock<D> {
  pub fn new(block: Executed<D>) -> Self {
    Self {
      block,
      votes: 0,
      voters: HashSet::new(),
    }
  }
}

/// Represents a single block in the volatile blockchain fork tree.
///
/// Ideally under perfect network conditions and abscence of failures
/// this structure would be a linked list of blocks.
///
/// However due to network delays, partitions or malicious actors some
/// blocks might be missed by some validators and proposers might start
/// building new blocks off an older block, and that creates several
/// histories.
///
/// This data structure represents all known blockchain
/// histories since the last finalized block.
#[derive(Debug, Clone)]
pub struct TreeNode<D: BlockData> {
  pub value: VolatileBlock<D>,
  pub parent: Option<*const TreeNode<D>>,
  pub children: Vec<TreeNode<D>>,
}

// SAFETY: No one beside us has the raw pointer, so we can safely
// transfer the TreeNode to another Thread when D can be safely transferred.
unsafe impl<D: BlockData> Send for TreeNode<D> where D: Send {}

// SAFETY: Discuss this with Karim
unsafe impl<D: BlockData> Sync for TreeNode<D> where D: Sync {}

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
  pub fn get(&self, hash: &Multihash) -> Option<&Self> {
    if self.value.block.hash().expect("previously verified") == *hash {
      Some(self)
    } else {
      for child in self.children.iter() {
        if let Some(b) = child.get(hash) {
          return Some(b);
        }
      }
      None
    }
  }

  /// Returns a mutable reference to a block with a given hash
  /// in the current subtree, or None if no such block is found.
  ///
  /// SAFETY: This struct and its methods are internal to this module
  /// and the node pointed to by the returned poineter is never reclaimed
  /// while reading the value retuned.
  pub fn get_mut(&mut self, hash: &Multihash) -> Option<*mut Self> {
    if self.value.block.hash().expect("previously veriefied") == *hash {
      Some(self)
    } else {
      let mut output = None;
      for child in self.children.iter_mut() {
        if let Some(b) = child.get_mut(hash) {
          output = Some(b);
          break;
        }
      }
      output
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
  pub fn head(&self) -> &Self {
    if self.children.is_empty() {
      return self; // leaf block
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
          // if two blocks have the same number of votes,
          // select the one with the longest chain.
          if subtree.depth() > top_subtree.depth() {
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
  pub fn add_child(&mut self, block: VolatileBlock<D>) {
    assert!(block.block.parent().unwrap() == self.value.block.hash().unwrap());

    // set parent link to ourself
    let block = TreeNode {
      value: block,
      parent: Some(self as *const Self),
      children: vec![],
    };

    // insert the block into this fork subtree as a leaf
    self.children.push(block);
  }

  /// Applies votes to a block, and all its ancestors until the
  /// last finalized block that is used as the justification for
  /// this vote.
  pub fn add_votes(&mut self, votes: u64, voter: Pubkey) {
    // apply those votes to the current block, but don't duplicate
    // validator votes on the same block.
    if self.value.voters.insert(voter) {
      self.value.votes += votes;
    }

    // also apply those votes to all the parent votes
    // until the justification point.
    let mut current = self;
    while let Some(ancestor) = current.parent {
      let ancestor = unsafe { &mut *(ancestor as *mut Self) as &mut Self };
      if ancestor.value.voters.insert(voter) {
        ancestor.value.votes += votes;
      }
      current = ancestor;
    }
  }

  /// The distance of this node from the root of the tree.
  /// This is used in determining the longest current chain.
  pub fn depth(&self) -> usize {
    self.path().count() - 1
  }

  /// Creates an iterator that walks the path from the current
  /// node until the last finalized block.
  pub fn path(&self) -> impl Iterator<Item = &TreeNode<D>> {
    PathIter::new(self)
  }

  pub fn is_descendant_of(&self, hash: &Multihash) -> bool {
    for step in self.path().skip(1) {
      if step.value.hash().unwrap() == *hash {
        return true;
      }
    }
    false
  }

  /// Returns the oldest ancestor of the current block
  /// that is still in the same epoch as this block.
  ///
  /// In other words: returns the first block in the epoch
  /// that contains the current block.
  ///
  /// This is used to check for finality of a block, and
  /// checking if the two consecutive epoch checkpoints
  /// are finalized.
  pub fn epoch_start(&self, epoch_blocks: u64) -> &TreeNode<D> {
    let epoch = |n: &TreeNode<D>| n.value.height() / epoch_blocks;
    let mut needle = self;
    for step in self.path().skip(1) {
      if epoch(step) == epoch(self) {
        needle = step;
      } else {
        break;
      }
    }
    if needle.value.hash().unwrap() != self.value.hash().unwrap() {
      needle
    } else {
      self
    }
  }

  /// Returns a state object that gives access to the entire
  /// state of this block and all its parents up to the root
  /// of the unfinalized state.
  pub fn state<'s>(&'s self) -> CascadingState<'s, D> {
    CascadingState::<'s, D> {
      iterator: PathIter::new(self),
    }
  }
}

#[derive(Debug)]
struct PathIter<'c, D: BlockData> {
  current: Option<&'c TreeNode<D>>,
}

impl<'c, D: BlockData> PathIter<'c, D> {
  pub fn new(current: &'c TreeNode<D>) -> Self {
    Self {
      current: Some(current),
    }
  }
}

impl<'c, D: BlockData> Clone for PathIter<'c, D> {
  fn clone(&self) -> Self {
    Self {
      current: self.current,
    }
  }
}

impl<'c, D: BlockData> Iterator for PathIter<'c, D> {
  type Item = &'c TreeNode<D>;

  fn next(&mut self) -> Option<Self::Item> {
    if let Some(node) = self.current {
      self.current = node.parent.map(|p| unsafe { &*p as &_ });
      Some(node)
    } else {
      None
    }
  }
}

/// This is a read-only view of a state of a forktree from a node
/// up until the root. The idea here is that every block in the fork
/// tree accumulates state changes. This view of the combined state
/// is passed to the blocks that are about to be attached to the current
/// head.
///
/// Whenever asked for an account data, it will traverse the tree upwards
/// until the first node returns a value for the requested address or none
/// if it wasn't found and it should be retreived from the finalized state.
pub struct CascadingState<'c, D: BlockData> {
  iterator: PathIter<'c, D>,
}

impl<'c, D: BlockData> State for CascadingState<'c, D> {
  fn get(&self, address: &Pubkey) -> Option<Account> {
    for current in self.iterator.clone() {
      if let Some(value) = current.value.block.state().get(address) {
        return Some(value);
      }
    }
    None
  }

  fn set(
    &mut self,
    _address: Pubkey,
    _account: Account,
  ) -> Result<Option<Account>, StateError> {
    Err(StateError::WritesNotSupported)
  }

  fn remove(&mut self, _address: Pubkey) -> Result<(), StateError> {
    Err(StateError::WritesNotSupported)
  }

  fn hash(&self) -> Multihash {
    unimplemented!() // not applicable here
  }
}

#[cfg(test)]
mod tests {
  use {
    super::{TreeNode, VolatileBlock},
    crate::{
      consensus::{
        block::{Block, BlockData, Produced},
        genesis::Limits,
        validator::Validator,
        Genesis,
      },
      primitives::Keypair,
      vm::{self, Executable, Executed, StateDiff},
    },
    chrono::Utc,
    ed25519_dalek::{PublicKey, SecretKey},
    multihash::Multihash,
    std::{
      collections::BTreeMap,
      marker::PhantomData,
      sync::Arc,
      time::Duration,
    },
  };

  fn generate_child<D: BlockData>(
    keypair: &Keypair,
    parent: &Produced<D>,
    data: D,
    vm: &vm::Machine,
  ) -> Executed<D> {
    Executed::new(
      &StateDiff::default(),
      Arc::new(
        Produced::new(
          keypair,
          parent.height + 1,
          parent.hash().unwrap(),
          data,
          *vec![].execute(vm, &StateDiff::default()).unwrap().hash(),
          vec![],
        )
        .unwrap(),
      ),
      vm,
    )
    .unwrap()
  }
  #[test]
  fn forktree_smoke() {
    let secret = SecretKey::from_bytes(&[
      157, 97, 177, 157, 239, 253, 90, 96, 186, 132, 74, 244, 146, 236, 44,
      196, 68, 73, 197, 105, 123, 50, 105, 25, 112, 59, 172, 3, 28, 174, 127,
      96,
    ])
    .unwrap();

    let public: PublicKey = (&secret).into();
    let keypair: Keypair = ed25519_dalek::Keypair { secret, public }.into();

    let genesis = Genesis::<u8> {
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

    let vm = vm::Machine::new(&genesis).unwrap();

    // blocks have no txs, so the statehash won't change across
    // blocks, but it needs to be a valid hash otherwise the block
    // gets rejected and not appended to the chain.
    let statehash = *vec![].execute(&vm, &StateDiff::default()).unwrap().hash();

    let produced = Arc::new(
      Produced::new(&keypair, 1, Multihash::default(), 1u8, statehash, vec![])
        .unwrap(),
    );

    let executed = Executed::new(&StateDiff::default(), produced, &vm).unwrap();
    let mut root = TreeNode::new(VolatileBlock::new(executed));
    let root_hash = root.value.hash().unwrap();
    let h1 = root.head();

    assert_eq!(h1.value.hash().unwrap(), root_hash);

    let child1 = generate_child(&keypair, &root.value, 2u8, &vm);
    let child1_1 = generate_child(&keypair, &child1, 11u8, &vm);
    let child1_2 = generate_child(&keypair, &child1, 12u8, &vm);

    let child2 = generate_child(&keypair, &root.value, 3u8, &vm);
    let child2_1 = generate_child(&keypair, &child2, 31u8, &vm);
    let child2_2 = generate_child(&keypair, &child2, 32u8, &vm);

    let child1_hash = child1.hash().unwrap();
    let child1_1_hash = child1_1.hash().unwrap();
    let child1_2_hash = child1_2.hash().unwrap();

    let child2_hash = child2.hash().unwrap();
    let child2_1_hash = child2_1.hash().unwrap();
    let child2_2_hash = child2_2.hash().unwrap();

    root.add_child(VolatileBlock::new(child1));
    let c1 = root.children.last_mut().unwrap();
    c1.add_child(VolatileBlock::new(child1_1));
    c1.add_child(VolatileBlock::new(child1_2));

    root.add_child(VolatileBlock::new(child2));
    let c2 = root.children.last_mut().unwrap();
    c2.add_child(VolatileBlock::new(child2_1));
    c2.add_child(VolatileBlock::new(child2_2));

    let get1 = root.get(&child1_hash).unwrap();
    let get11 = root.get(&child1_1_hash).unwrap();
    let get12 = root.get(&child1_2_hash).unwrap();

    let get2 = root.get(&child2_hash).unwrap();
    let get21 = root.get(&child2_1_hash).unwrap();
    let get22 = root.get(&child2_2_hash).unwrap();

    let get3 = root.get(&Multihash::default());

    let c1 = root.children.first().unwrap();
    let c11 = c1.children.first().unwrap();
    let c12 = c1.children.last().unwrap();

    let c2 = root.children.last().unwrap();
    let c21 = c2.children.first().unwrap();
    let c22 = c2.children.last().unwrap();

    assert_eq!(root.value.hash().unwrap(), root_hash);
    assert_eq!(get1.value.hash().unwrap(), c1.value.hash().unwrap());
    assert_eq!(get11.value.hash().unwrap(), c11.value.hash().unwrap());
    assert_eq!(get12.value.hash().unwrap(), c12.value.hash().unwrap());

    assert_eq!(get2.value.hash().unwrap(), c2.value.hash().unwrap());
    assert_eq!(get21.value.hash().unwrap(), c21.value.hash().unwrap());
    assert_eq!(get22.value.hash().unwrap(), c22.value.hash().unwrap());

    assert!(get3.is_none());

    assert_eq!(get11.depth(), 2);
    assert_eq!(get12.depth(), 2);
    assert_eq!(get1.depth(), 1);

    assert_eq!(get21.depth(), 2);
    assert_eq!(get22.depth(), 2);
    assert_eq!(get2.depth(), 1);

    assert_eq!(root.depth(), 0);
  }
}
