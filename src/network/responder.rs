use {
  futures::Stream,
  rand::{distributions::Uniform, thread_rng, Rng},
  std::{
    collections::{hash_map::Entry, HashMap},
    hash::Hash,
    pin::Pin,
    task::{Context, Poll},
    time::{Duration, Instant},
  },
};

/// This is a generic mechanism for responding to one-off events
/// by a swarm of peers. It is used for things like block replay
/// requests, slashing evidence, state CID request.
///
/// The idea behind this type is that we have a group of validators,
/// potentially in the 1000s of them, and one or some of them need
/// to request some information from other validators, but they don't
/// know which ones have the response, and we want to avoid an avalanche
/// of responses from every validator that has it.
///
/// Whenever a request is observed on the p2p gossip, each validator will
/// wait a random interval between [slot_time, N * slot_time] miliseconds,
/// where N = log2(total_validator_count). Then respond to that request.
///
/// If it observes that a response has already been served by some other
/// peer, then the request gets discarded and considered as fulfilled.
///
/// If the same request arrives from multiple peers while the responder is
/// waiting the random interval, its coalesced and merged into one request.
pub struct SwarmResponder<R: Eq + Hash + Copy> {
  low: Duration,
  high: Duration,
  requests: HashMap<R, Instant>,
}

impl<R: Eq + Hash + Copy> SwarmResponder<R> {
  pub fn new(slot: Duration, network_size: usize) -> Self {
    Self {
      low: slot, // wait for at least 1 slot time, to minimize duplicates
      // spread the delay propotionally to network size
      high: slot * (network_size as f64).log2().round() as u32,
      requests: HashMap::new(),
    }
  }

  /// Registers a new request.
  ///
  /// If this request is already registered then it will be discarded,
  /// and the original random interval is used for the first registered
  /// request.
  pub fn request(&mut self, req: R) {
    if let Entry::Vacant(e) = self.requests.entry(req) {
      let distribution = Uniform::new(self.low, self.high);
      let delay = thread_rng().sample(distribution);
      let response_at = Instant::now() + delay;
      e.insert(response_at);
    }
  }

  /// Cancels a pending request if it was already served by some other peer.
  pub fn cancel(&mut self, req: &R) {
    self.requests.remove(req);
  }
}

impl<R: Eq + Hash + Copy> Unpin for SwarmResponder<R> {}
impl<R: Eq + Hash + Copy> Stream for SwarmResponder<R> {
  type Item = R;

  fn poll_next(
    mut self: Pin<&mut Self>,
    _: &mut Context<'_>,
  ) -> Poll<Option<Self::Item>> {
    let now = Instant::now();
    let ready = self
      .requests
      .iter()
      .find(|(_, at)| now >= **at)
      .map(|(k, _)| *k);
    if let Some(req) = ready {
      self.requests.remove(&req);
      return Poll::Ready(Some(req));
    }
    Poll::Pending
  }
}
