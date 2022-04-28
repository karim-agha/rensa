use {
  crate::{consensus::BlockData, vm::Executed},
  futures::future::join_all,
  serde::{Deserialize, Serialize},
  std::sync::Arc,
  tokio::sync::mpsc::{error::SendError, unbounded_channel, UnboundedSender},
};

/// Specifies the commitment level of a block.
/// Commitment levels are assurance levels that guarantee
/// that the block will be part of the canonical chain and
/// end up being finalized. See the consensus module for
/// a detailed explanation of those levels and stages of a
/// block processing.
#[derive(Debug, Copy, Clone, Serialize, Deserialize, PartialEq)]
pub enum Commitment {
  Included,
  Confirmed,
  Finalized,
}

/// This trait is implemented by all services and components that ingest
/// blocks as soon as the consensus engine agrees on them. This includes
/// things like disk persistance, RPC service, sync service, etc.
#[async_trait::async_trait]
pub trait BlockConsumer<D: BlockData>: Sync + Send {
  async fn consume(&self, block: Arc<Executed<D>>, commitment: Commitment);
}

/// A collection of block consumers that will all receive
/// a reference to all newly included, committed and finalized blocks.
pub struct BlockConsumers<D: BlockData> {
  sender: UnboundedSender<(Arc<Executed<D>>, Commitment)>,
}

impl<D: BlockData> BlockConsumers<D> {
  pub fn new(consumers: Vec<Arc<dyn BlockConsumer<D>>>) -> Self {
    // move all the consumption heavyweight work to a sperate thread
    let (sender, mut receiver) =
      unbounded_channel::<(Arc<Executed<D>>, Commitment)>();
    tokio::spawn(async move {
      while let Some((b, c)) = receiver.recv().await {
        join_all(consumers.iter().map(|consumer| {
          let block = Arc::clone(&b);
          let consumer = Arc::clone(consumer);
          tokio::spawn(async move {
            consumer.consume(block, c).await;
          })
        }))
        .await;
      }
    });

    Self { sender }
  }

  pub fn consume(
    &self,
    block: Arc<Executed<D>>,
    commitment: Commitment,
  ) -> Result<(), SendError<(Arc<Executed<D>>, Commitment)>> {
    self.sender.send((block, commitment))
  }
}
