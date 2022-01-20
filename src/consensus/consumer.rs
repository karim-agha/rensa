use super::{block::{self, BlockData}, chain::Chain};
use std::marker::PhantomData;
use tracing::info;

pub struct BlockConsumer<D: BlockData>(PhantomData<D>);

impl<D: BlockData> BlockConsumer<D> {
  pub fn new(_genesis: &Chain<D>) -> Self {
    BlockConsumer(PhantomData)
  }

  pub fn consume(&mut self, block: block::Produced<D>) {
    info!("consuming block {block:?}");
  }
}
