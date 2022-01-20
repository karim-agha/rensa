use super::block;
use futures::Stream;
use serde::{Deserialize, Serialize};
use std::{
  marker::PhantomData,
  pin::Pin,
  task::{Context, Poll},
};

pub struct BlockProducer<D>(PhantomData<D>)
where
  D: Eq + Serialize + for<'a> Deserialize<'a>;

impl<D> BlockProducer<D>
where
  D: Eq + Serialize + for<'a> Deserialize<'a>,
{
  pub fn new(_genesis: &block::Genesis<D>) -> Self {
    BlockProducer(PhantomData)
  }

  pub fn produce(&self, _slot: u64) {}
}

impl<D> Stream for BlockProducer<D>
where
  D: Eq + Serialize + for<'a> Deserialize<'a>,
{
  type Item = block::Produced<D>;

  fn poll_next(
    self: Pin<&mut Self>,
    _: &mut Context<'_>,
  ) -> Poll<Option<Self::Item>> {
    Poll::Pending
  }
}
