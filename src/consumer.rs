use {
  crate::{consensus::BlockData, vm::Executed},
  tokio::sync::mpsc::{error::SendError, UnboundedSender},
};

/// Specifies the commitment level of a block.
/// Commitment levels are assurance levels that guarantee
/// that the block will be part of the canonical chain and
/// end up being finalized. See the consensus module for
/// a detailed explanation of those levels and stages of a
/// block processing.
#[derive(Debug, Copy, Clone)]
pub enum Commitment {
  Included,
  Confirmed,
  Finalized,
}

/// This trait is implemented by all services and components that ingest
/// blocks as soon as the consensus engine agrees on them. This includes
/// things like disk persistance, RPC service, sync service, etc.
pub trait BlockConsumer<D: BlockData>: Sync + Send {
  fn consume(&self, block: &Executed<D>, commitment: Commitment);
}

/// A collection of block consumers that will all receive
/// a reference to all newly included, committed and finalized blocks.
pub struct BlockConsumers<D: BlockData> {
  sender: UnboundedSender<(Executed<D>, Commitment)>,
}

impl<D: BlockData> BlockConsumers<D> {
  pub fn new(consumers: Vec<Box<dyn BlockConsumer<D>>>) -> Self {
    // move all the consumption heavyweight work to a sperate thread
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
      while let Some((b, c)) = receiver.recv().await {
        for consumer in consumers.iter() {
          consumer.consume(&b, c);
        }
      }
    });

    Self { sender }
  }

  pub fn consume(
    &self,
    block: Executed<D>,
    commitment: Commitment,
  ) -> Result<(), SendError<(Executed<D>, Commitment)>> {
    self.sender.send((block, commitment))
  }
}
