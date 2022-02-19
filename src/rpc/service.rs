use {
  super::ApiEvent,
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

pub struct ApiService {
  out_events: VecDeque<ApiEvent>,
}

impl ApiService {
  pub fn new(addrs: Vec<SocketAddr>) -> Self {
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
      out_events: addrs
        .into_iter()
        .map(ApiEvent::ServiceInitialized)
        .collect(),
    }
  }
}

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
