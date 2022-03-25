use {
  crate::{
    consensus::{Block, Genesis},
    consumer::Commitment,
    primitives::{Account, Pubkey, ToBase58String},
    storage::{BlockStore, PersistentState},
    vm::{State, Transaction},
  },
  axum::{
    extract::{Extension, Path, Query},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json,
    Router,
  },
  axum_extra::response::ErasedJson,
  futures::Stream,
  indexmap::IndexMap,
  multihash::Multihash,
  serde::Deserialize,
  serde_json::json,
  std::{
    collections::HashMap,
    net::SocketAddr,
    pin::Pin,
    str::FromStr,
    sync::Arc,
    task::{Context, Poll},
  },
  tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender},
};

type BlockType = Vec<Transaction>;

struct ServiceSharedState {
  state: PersistentState,
  blocks: BlockStore,
  genesis: Genesis<BlockType>,
  sender: UnboundedSender<Transaction>,
}

pub struct ApiService {
  receiver: UnboundedReceiver<Transaction>,
}

impl ApiService {
  pub fn new(
    addrs: Vec<SocketAddr>,
    state: PersistentState,
    blocks: BlockStore,
    genesis: Genesis<BlockType>,
  ) -> Self {
    let (sender, receiver) = mpsc::unbounded_channel();
    let shared_state = Arc::new(ServiceSharedState {
      state,
      blocks,
      genesis,
      sender,
    });

    let svc = Router::new()
      .route("/info", get(serve_info))
      .route("/block/:height", get(serve_block))
      .route("/account/:account", get(serve_account))
      .route("/transaction/:hash", get(serve_transaction))
      .route("/transaction", post(serve_send_transaction))
      .layer(Extension(shared_state));

    addrs.iter().cloned().for_each(|addr| {
      let svc = svc.clone();
      tokio::spawn(async move {
        axum::Server::bind(&addr)
          .serve(svc.into_make_service())
          .await
          .unwrap();
      });
    });

    Self { receiver }
  }
}

impl Unpin for ApiService {}
impl Stream for ApiService {
  type Item = Transaction;

  fn poll_next(
    mut self: Pin<&mut Self>,
    cx: &mut Context<'_>,
  ) -> Poll<Option<Self::Item>> {
    self.receiver.poll_recv(cx)
  }
}

/// Examples:
///  - /transaction/W1kbxQHfhfkG3DYoSczbbie4L16XviszWT4yK4VzUBW1Yy
async fn serve_transaction(
  Path(hash): Path<TransactionHash>,
  Extension(state): Extension<Arc<ServiceSharedState>>,
) -> impl IntoResponse {
  if let Some(tx) = state.blocks.get_transaction(&hash) {
    (
      StatusCode::OK,
      ErasedJson::pretty(json!({
        "hash": tx.transaction.hash().to_b58(),
        "block": tx.block,
        "commitment": state.blocks.get_block_commitment(tx.block),
        "transaction": tx.transaction,
        "output": tx.output.map(|o| o.into_iter().collect::<IndexMap<_, _>>())
      })),
    )
  } else {
    (
      StatusCode::NOT_FOUND,
      ErasedJson::pretty(json! ({
        "transaction": hash.to_b58(),
        "error": "not_found"
      })),
    )
  }
}

/// Examples:
///  - /accounts/B5Vsy6UPyGopvAM2GFv9VMyn29As8wjGyMxCQMVAGH6A
///  - /accounts/B5Vsy6UPyGopvAM2GFv9VMyn29As8wjGyMxCQMVAGH6A?
///    commitment=confirmed
async fn serve_account(
  Path(account): Path<Pubkey>,
  Extension(state): Extension<Arc<ServiceSharedState>>,
  Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
  let commitment = extract_commitment(params);

  let make_response = |acc: Account, commitment: Commitment| {
    (
      StatusCode::OK,
      ErasedJson::pretty(json! ({
        "account": {
          "address": account,
          "nonce": acc.nonce,
          "owner": acc.owner,
          "data": acc.data.map(|a| a.to_b58())
        },
        "commitment": commitment
      })),
    )
  };

  if let Commitment::Confirmed = commitment {
    if let Some(acc) = get_confirmed_account(&account, state.as_ref()) {
      return make_response(acc, commitment);
    }
  }

  if let Some(acc) = state.state.get(&account) {
    return make_response(acc, Commitment::Finalized);
  }

  (
    StatusCode::NOT_FOUND,
    ErasedJson::pretty(json! ({
      "account": account,
      "error": "not_found"
    })),
  )
}

async fn serve_info(
  Extension(state): Extension<Arc<ServiceSharedState>>,
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
  Extension(state): Extension<Arc<ServiceSharedState>>,
) -> impl IntoResponse {
  if let Some((block, commitment)) = state.blocks.get_by_height(height) {
    (
      StatusCode::OK,
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
      })),
    )
  } else {
    (
      StatusCode::NOT_FOUND,
      ErasedJson::pretty(json!({
        "error": "not found",
      })),
    )
  }
}

async fn serve_send_transaction(
  Json(transaction): Json<Transaction>,
  Extension(state): Extension<Arc<ServiceSharedState>>,
) -> impl IntoResponse {
  // filter out outsized transactions at the RPC level.
  if let Err(e) = transaction.verify_limits(&state.genesis.limits) {
    return (
      StatusCode::BAD_REQUEST,
      ErasedJson::pretty(json! ({
        "error": e,
      })),
    );
  }

  // filter out invalid signatures and addresses at the
  // RPC level before bothering p2p and consensus and
  // other validators.
  if let Err(e) = transaction.verify_signatures() {
    return (
      StatusCode::BAD_REQUEST,
      ErasedJson::pretty(json! ({
        "error": e,
      })),
    );
  }

  // filter out transaction replays at the RPC level
  let expected_nonce =
    get_confirmed_account(&transaction.payer, state.as_ref())
      .or_else(|| state.state.get(&transaction.payer))
      .map(|a| a.nonce)
      .unwrap_or(0)
      + 1;

  if transaction.nonce != expected_nonce {
    return (
      StatusCode::BAD_REQUEST,
      ErasedJson::pretty(json! ({
        "error": format!(
          "invalid nonce. expected {}, found {}",
          expected_nonce, transaction.nonce
        ),
      })),
    );
  }

  let hash = *transaction.hash();
  if let Err(e) = state.sender.send(transaction) {
    return (
      StatusCode::INTERNAL_SERVER_ERROR,
      ErasedJson::pretty(json! ({
        "error": e.to_string(),
      })),
    );
  }

  (
    StatusCode::CREATED,
    ErasedJson::pretty(json! ({
      "transaction": hash.to_b58(),
    })),
  )
}

fn extract_commitment(params: HashMap<String, String>) -> Commitment {
  match params.get("commitment") {
    None => Commitment::Finalized,
    Some(value) => match value.to_lowercase().as_str() {
      "finalized" => Commitment::Finalized,
      "confirmed" => Commitment::Confirmed,
      _ => Commitment::Finalized,
    },
  }
}

/// Tries to retreive an account from the state generated by
/// confirmed blocks, that have not been finalized yet.
fn get_confirmed_account(
  account: &Pubkey,
  state: &ServiceSharedState,
) -> Option<Account> {
  let latest_finalized = state.blocks.latest(Commitment::Finalized);
  let latest_confirmed = state.blocks.latest(Commitment::Confirmed);

  if let Some(finalized) = latest_finalized {
    let finalized_height = finalized.height();
    if let Some(confirmed) = latest_confirmed {
      let confirmed_height = confirmed.height();

      // walk the confirmed blocks one by one until
      // it reaches the finalized height.
      let mut cursor = confirmed_height;
      while cursor > finalized_height {
        if let Some((block, _)) = state.blocks.get_by_height(cursor) {
          if let Some(acc) = block.state().get(account) {
            return Some(acc);
          } else {
            cursor -= 1;
          }
        }
      }
    }
  }

  None
}

struct TransactionHash(Multihash);
impl std::ops::Deref for TransactionHash {
  type Target = Multihash;

  fn deref(&self) -> &Self::Target {
    &self.0
  }
}

impl FromStr for TransactionHash {
  type Err = StatusCode;

  fn from_str(s: &str) -> Result<Self, Self::Err> {
    Ok(TransactionHash(
      Multihash::from_bytes(
        &bs58::decode(s)
          .into_vec()
          .map_err(|_| StatusCode::BAD_REQUEST)?,
      )
      .map_err(|_| StatusCode::BAD_REQUEST)?,
    ))
  }
}

impl<'de> Deserialize<'de> for TransactionHash {
  fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
  where
    D: serde::Deserializer<'de>,
  {
    TransactionHash::from_str(String::deserialize(deserializer)?.as_str())
      .map_err(|_| serde::de::Error::custom("invalid transaction hash"))
  }
}
