use super::{block::{self, BlockData}, validator::Validator};

pub struct Chain<'g, D: BlockData> {
  genesis: &'g block::Genesis<D>,
}

impl<'g, D: BlockData> Chain<'g, D> {
  pub fn new(genesis: &'g block::Genesis<D>) -> Self {
    Self { genesis }
  }

  pub fn genesis(&self) -> &'g block::Genesis<D> {
    self.genesis
  }

  pub fn last_finalized(&self) -> &'g impl block::Block<D> {
    self.genesis
  }

  pub fn validators(&self) -> &'g [Validator] {
    &self.genesis.validators
  }
}
