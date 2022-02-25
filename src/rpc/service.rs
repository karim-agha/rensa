use {
  super::ApiEvent,
  crate::{
    consensus::{BlockData, Genesis},
    consumer::{BlockConsumer, Commitment},
    storage::{BlockStore, PersistentState},
    vm::Executed,
  },
  axum::{
    routing::{get, post},
    Router,
  },
  axum_extra::response::ErasedJson,
  futures::Stream,
  serde_json::json,
  std::{
    collections::VecDeque,
    net::SocketAddr,
    pin::Pin,
    task::{Context, Poll},
  },
};

pub struct ApiService<D: BlockData> {
  _state: PersistentState,
  _blocks: BlockStore<D>,
  out_events: VecDeque<ApiEvent>,
}

impl<D: BlockData> ApiService<D> {
  pub fn new(
    addrs: Vec<SocketAddr>,
    state: PersistentState,
    blocks: BlockStore<D>,
    genesis: Genesis<D>,
  ) -> Self {
    let blocksc = blocks.clone();
    let svc = Router::new()
      .route(
        "/about",
        get(|| async move {
          let height = blocksc
            .latest(Commitment::Finalized)
            .map(|b| b.height)
            .unwrap_or(0);

          ErasedJson::pretty(json! ({
            "system": {
              "name": "Rensa",
              "version": env!("CARGO_PKG_VERSION")
            },
            "finalized": {
              "height": height,
            },
            "genesis": genesis,
          }))
        }),
      )
      .route(
        "/send_transaction",
        post(|| async {
          ErasedJson::pretty(json! ({
            "status": "not_implemented"
          }))
        }),
      );

    addrs.iter().cloned().for_each(|addr| {
      let svc = svc.clone();
      tokio::spawn(async move {
        axum::Server::bind(&addr)
          .serve(svc.into_make_service())
          .await
          .unwrap();
      });
    });

    Self {
      _blocks: blocks,
      _state: state,
      out_events: addrs
        .into_iter()
        .map(ApiEvent::ServiceInitialized)
        .collect(),
    }
  }
}

impl<D: BlockData> BlockConsumer<D> for ApiService<D> {
  fn consume(&self, _block: &Executed<D>, _commitment: Commitment) {
    // todo: ingest confirmed but not finalized blocks
  }
}

impl<D: BlockData> Unpin for ApiService<D> {}
impl<D: BlockData> Stream for ApiService<D> {
  type Item = ApiEvent;

  fn poll_next(
    mut self: Pin<&mut Self>,
    _: &mut Context<'_>,
  ) -> Poll<Option<Self::Item>> {
    if let Some(event) = self.out_events.pop_front() {
      return Poll::Ready(Some(event));
    }
    Poll::Pending
  }
}
