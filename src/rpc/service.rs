use {
  super::ApiEvent,
  crate::storage::PersistentState,
  axum::{
    routing::{get, post},
    Json,
    Router,
  },
  futures::Stream,
  serde_json::json,
  std::{
    collections::VecDeque,
    net::SocketAddr,
    pin::Pin,
    task::{Context, Poll},
  },
};

pub struct ApiService<'s> {
  _storage: &'s PersistentState,
  out_events: VecDeque<ApiEvent>,
}

impl<'s> ApiService<'s> {
  pub fn new(addrs: Vec<SocketAddr>, storage: &'s PersistentState) -> Self {
    let svc = Router::new()
      .route(
        "/about",
        get(|| async {
          Json(json! ({
            "system": {
              "name": "Rensa",
              "version": env!("CARGO_PKG_VERSION")
            }
          }))
        }),
      )
      .route(
        "/send_transaction",
        post(|| async {
          Json(json! ({
            "status": "accepted"
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
      _storage: storage,
      out_events: addrs
        .into_iter()
        .map(ApiEvent::ServiceInitialized)
        .collect(),
    }
  }
}

impl Stream for ApiService<'_> {
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
