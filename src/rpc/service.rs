use {
  super::ApiEvent,
  futures::Stream,
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
