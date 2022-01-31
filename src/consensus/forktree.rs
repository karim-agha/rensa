use super::block::{self, Block, BlockData};
use crate::primitives::Pubkey;
use multihash::Multihash;
use std::{
  cell::RefCell, cmp::Ordering, collections::HashSet, ops::Deref, rc::Rc,
};

/// A block that is still not finalized and its votes
/// are still being counted.
///
/// Those blocks are not guaranteed to never be
/// discarded by the blockchain yet until they are
/// voted on and finalized.
#[derive(Debug, Clone)]
pub struct VolatileBlock<D: BlockData> {
  pub block: block::Produced<D>,
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
  pub fn new(block: block::Produced<D>) -> Self {
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
  pub parent: Option<*mut TreeNode<D>>,
  pub children: Vec<Rc<RefCell<TreeNode<D>>>>,
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
  #[cfg(test)]
  fn get(&self, hash: &Multihash) -> Option<*const Self> {
    if self.value.block.hash().expect("previously verified") == *hash {
      Some(self)
    } else {
      for child in self.children.iter() {
        let child = child.borrow();
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
  pub fn head(&self) -> *const Self {
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
    top_subtree.borrow().head() as *const Self
  }

  /// Given a hash of a block in this subtree this method
  /// will locate and move out that block and all its children
  /// from the tree as a separate tree.
  ///
  /// This is used when finalizing a block in the volatile state.
  pub fn _take(&mut self, hash: &Multihash) -> Option<Self> {
    if let Some(node) = self.get_mut(hash) {
      let node = unsafe { &mut *node as &mut Self };
      if let Some(parent) = node.parent {
        let parent = unsafe { &mut *parent as &mut Self };
        for (index, child) in parent.children.iter().enumerate() {
          if child.borrow().value.hash().unwrap() != node.value.hash().unwrap()
          {
            continue; // keep looking
          }

          // found it
          child.borrow_mut().parent = None;
          return Some(parent.children.remove(index).borrow().clone());
        }
      }
    }

    None
  }

  /// Adds an immediate child to this forktree node.
  pub fn add_child(
    &mut self,
    block: VolatileBlock<D>,
  ) -> Rc<RefCell<TreeNode<D>>> {
    assert!(block.block.parent().unwrap() == self.value.block.hash().unwrap());

    let blockhash = block.block.hash().unwrap();
    for child in self.children.iter() {
      if child.borrow().value.block.hash().unwrap() == blockhash {
        return Rc::clone(child);
      }
    }

    // set parent link to ourself
    let block = Rc::new(RefCell::new(TreeNode {
      value: block,
      parent: Some(self as *mut Self),
      children: vec![],
    }));

    // insert the block into this fork subtree as a leaf
    let ret = Rc::clone(&block);
    self.children.push(block);
    ret
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

#[cfg(test)]
mod tests {
  use ed25519_dalek::{PublicKey, SecretKey};
  use multihash::Multihash;

  use super::{TreeNode, VolatileBlock};
  use crate::{
    consensus::block::{Block, BlockData, Produced},
    primitives::Keypair,
  };

  fn generate_child<D: BlockData>(
    keypair: &Keypair,
    parent: &Produced<D>,
    data: D,
  ) -> Produced<D> {
    Produced::new(
      keypair,
      parent.height + 1,
      parent.hash().unwrap(),
      data,
      vec![],
    )
    .unwrap()
  }

  #[test]
  fn forktree_smoke() {
    let secret = SecretKey::from_bytes(&[
      157, 097, 177, 157, 239, 253, 090, 096, 186, 132, 074, 244, 146, 236,
      044, 196, 068, 073, 197, 105, 123, 050, 105, 025, 112, 059, 172, 003,
      028, 174, 127, 096,
    ])
    .unwrap();

    let public: PublicKey = (&secret).into();
    let keypair: Keypair = ed25519_dalek::Keypair { secret, public }.into();

    let mut root = TreeNode::new(VolatileBlock::new(
      Produced::new(&keypair, 1, Multihash::default(), 1u8, vec![]).unwrap(),
    ));

    let root_hash = root.value.hash().unwrap();

    let h1 = root.head();

    assert_eq!(
      unsafe { &*h1 as &TreeNode<u8> }.value.hash().unwrap(),
      root_hash
    );

    let child1 = generate_child(&keypair, &root.value, 2u8);
    let child1_1 = generate_child(&keypair, &child1, 11u8);
    let child1_2 = generate_child(&keypair, &child1, 12u8);

    let child2 = generate_child(&keypair, &root.value, 3u8);
    let child2_1 = generate_child(&keypair, &child2, 31u8);
    let child2_2 = generate_child(&keypair, &child2, 32u8);

    let child1_hash = child1.hash().unwrap();
    let child1_1_hash = child1_1.hash().unwrap();
    let child1_2_hash = child1_2.hash().unwrap();

    let child2_hash = child2.hash().unwrap();
    let child2_1_hash = child2_1.hash().unwrap();
    let child2_2_hash = child2_2.hash().unwrap();

    let c1 = root.add_child(VolatileBlock::new(child1));
    let c11 = c1.borrow_mut().add_child(VolatileBlock::new(child1_1));
    let c12 = c1.borrow_mut().add_child(VolatileBlock::new(child1_2));

    let c2 = root.add_child(VolatileBlock::new(child2));
    let c21 = c2.borrow_mut().add_child(VolatileBlock::new(child2_1));
    let c22 = c2.borrow_mut().add_child(VolatileBlock::new(child2_2));

    let get1 = unsafe { &*root.get(&child1_hash).unwrap() as &TreeNode<u8> };
    let get11 = unsafe { &*root.get(&child1_1_hash).unwrap() as &TreeNode<u8> };
    let get12 = unsafe { &*root.get(&child1_2_hash).unwrap() as &TreeNode<u8> };

    let get2 = unsafe { &*root.get(&child2_hash).unwrap() as &TreeNode<u8> };
    let get21 = unsafe { &*root.get(&child2_1_hash).unwrap() as &TreeNode<u8> };
    let get22 = unsafe { &*root.get(&child2_2_hash).unwrap() as &TreeNode<u8> };

    let get3 = root.get(&Multihash::default());

    assert_eq!(root.value.hash().unwrap(), root_hash);
    assert_eq!(
      get1.value.hash().unwrap(),
      c1.borrow().value.hash().unwrap()
    );
    assert_eq!(
      get11.value.hash().unwrap(),
      c11.borrow().value.hash().unwrap()
    );
    assert_eq!(
      get12.value.hash().unwrap(),
      c12.borrow().value.hash().unwrap()
    );

    assert_eq!(
      get2.value.hash().unwrap(),
      c2.borrow().value.hash().unwrap()
    );
    assert_eq!(
      get21.value.hash().unwrap(),
      c21.borrow().value.hash().unwrap()
    );
    assert_eq!(
      get22.value.hash().unwrap(),
      c22.borrow().value.hash().unwrap()
    );

    assert!(get3.is_none());

    assert_eq!(get11.depth(), 2);
    assert_eq!(get12.depth(), 2);
    assert_eq!(get1.depth(), 1);

    assert_eq!(get21.depth(), 2);
    assert_eq!(get22.depth(), 2);
    assert_eq!(get2.depth(), 1);

    assert_eq!(root.depth(), 0);

    let o2 = root._take(&child2_hash);

    assert!(o2.is_some());

    let o2u = o2.unwrap();
    assert_eq!(o2u.parent, None);
    assert_eq!(o2u.value.hash().unwrap(), child2_hash);

    // make sure its moved out and not in the original tree anymore
    assert!(root.get(&child2_hash).is_none());
  }
}
