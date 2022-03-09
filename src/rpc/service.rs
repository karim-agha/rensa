use {
  super::ApiEvent,
  crate::{
    consensus::{Block, BlockData, Genesis},
    consumer::Commitment,
    primitives::ToBase58String,
    storage::{BlockStore, PersistentState},
    vm::Transaction,
  },
  axum::{
    extract::{Extension, Path},
    response::IntoResponse,
    routing::{get, post},
    AddExtensionLayer,
    Router,
  },
  axum_extra::response::ErasedJson,
  futures::Stream,
  indexmap::IndexMap,
  serde_json::json,
  std::{
    collections::VecDeque,
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
  },
};

struct ServiceSharedState<D: BlockData> {
  _state: PersistentState,
  blocks: BlockStore<D>,
  genesis: Genesis<D>,
}

pub struct ApiService {
  out_events: VecDeque<ApiEvent>,
}

impl ApiService {
  pub fn new<D: BlockData>(
    addrs: Vec<SocketAddr>,
    state: PersistentState,
    blocks: BlockStore<D>,
    genesis: Genesis<D>,
  ) -> Self {
    let shared_state = Arc::new(ServiceSharedState {
      _state: state,
      blocks,
      genesis,
    });

    let svc = Router::new()
      .route("/info", get(serve_info::<D>))
      .route("/block/:height", get(serve_block))
      .route("/transaction", post(serve_send_transaction))
      .layer(AddExtensionLayer::new(shared_state));

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
      out_events: addrs
        .into_iter()
        .map(ApiEvent::ServiceInitialized)
        .collect(),
    }
  }
}

impl Unpin for ApiService {}
impl Stream for ApiService {
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

async fn serve_info<D: BlockData>(
  Extension(state): Extension<Arc<ServiceSharedState<D>>>,
) -> impl IntoResponse {
  let (fheight, fhash) = state
    .blocks
    .latest(Commitment::Finalized)
    .map(|b| (b.height, b.hash().unwrap()))
    .unwrap_or((0, state.genesis.hash().unwrap()));

  let (cheight, chash) = state
    .blocks
    .latest(Commitment::Confirmed)
    .map(|b| (b.height, b.hash().unwrap()))
    .unwrap_or((0, state.genesis.hash().unwrap()));

  ErasedJson::pretty(json! ({
    "system": {
      "name": "Rensa",
      "version": env!("CARGO_PKG_VERSION")
    },
    "finalized": {
      "height": fheight,
      "block": fhash.to_bytes().to_b58()
    },
    "confirmed": {
      "height": cheight,
      "block": chash.to_bytes().to_b58(),
    },
    "genesis": state.genesis,
  }))
}

async fn serve_block(
  Path(height): Path<u64>,
  Extension(state): Extension<Arc<ServiceSharedState<Vec<Transaction>>>>,
) -> impl IntoResponse {
  if let Some((block, commitment)) = state.blocks.get_by_height(height) {
    ErasedJson::pretty(json!({
      "commitment": commitment,
      "block": {
        "parent": block.underlying.parent.to_b58(),
        "state": block.underlying.state_hash.to_b58(),
        "height": block.underlying.height,
        "hash": block.underlying.hash().unwrap().to_b58(),
        "producer": block.underlying.signature.0,
        "signature": block.underlying.signature.1.to_b58(),
        "votes": block.underlying.votes,
        "transactions": block.underlying.data
          .iter()
          .map(|tx| (tx.hash().to_b58(), tx))
          .collect::<IndexMap<_, _>>()
      },
      "outputs": block.output.logs
        .iter()
        .map(|(txhash, logs)|
          (
            txhash.to_b58(),
            logs.iter().cloned().collect::<IndexMap<_, _>>())
          )
        .collect::<IndexMap<_, _>>(),
      "errors": block.output.errors
        .iter()
        .map(|(txhash, error)| (txhash.to_b58(), error))
        .collect::<IndexMap<_, _>>()
    }))
  } else {
    ErasedJson::pretty(json!({
      "error": "not found",
    }))
  }
}

async fn serve_send_transaction() -> impl IntoResponse {
  ErasedJson::pretty(json! ({
    "status": "not_implemented"
  }))
}
