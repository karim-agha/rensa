use super::{
  block::{self, BlockData},
  chain::Chain,
};
use futures::Stream;
use std::{
  marker::PhantomData,
  pin::Pin,
  task::{Context, Poll},
};

pub struct BlockProducer<D: BlockData>(PhantomData<D>);

impl<D: BlockData> BlockProducer<D> {
  pub fn new(_chain: &Chain<D>) -> Self {
    BlockProducer(PhantomData)
  }

  pub fn produce(&self, _slot: u64) {}
}

impl<D: BlockData> Stream for BlockProducer<D> {
  type Item = block::Produced<D>;

  fn poll_next(
    self: Pin<&mut Self>,
    _: &mut Context<'_>,
  ) -> Poll<Option<Self::Item>> {
    Poll::Pending
  }
}
