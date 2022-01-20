use super::block;
use serde::{Deserialize, Serialize};
use std::marker::PhantomData;

pub struct BlockConsumer<D>(PhantomData<D>)
where
  D: Eq + Serialize + for<'a> Deserialize<'a>;

impl<D> BlockConsumer<D>
where
  D: Eq + Serialize + for<'a> Deserialize<'a>,
{
  pub fn new(_genesis: &block::Genesis<D>) -> Self {
    BlockConsumer(PhantomData)
  }

  pub fn consume(&mut self, _block: block::Produced<D>) {}
}
