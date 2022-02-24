use {
  libp2p::core::PeerId,
  std::{sync::Arc, time::Duration},
};

#[derive(Clone)]
pub struct PeerAuthorizer(Arc<dyn Fn(&str, &PeerId) -> bool + Send + Sync>);

impl PeerAuthorizer {
  pub fn new<F>(predicate: F) -> Self
  where
    F: Fn(&str, &PeerId) -> bool + Send + Sync + 'static,
  {
    Self(Arc::new(predicate))
  }

  pub fn allow(&self, topic: &str, peer: &PeerId) -> bool {
    self.0(topic, peer)
  }
}

impl std::fmt::Debug for PeerAuthorizer {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_tuple("PeerAuthenticator").field(&"[impl]").finish()
  }
}

/// Configuration paramaters for Episub
#[derive(Debug, Clone)]
pub struct Config {
  /// Estimated number of online nodes joining one topic
  pub network_size: usize,

  /// HyParView Active View constant
  /// active view size = Ln(N) + C
  pub active_view_factor: usize,

  /// HyParView Passive View constant
  /// active view size = C * Ln(N)
  pub passive_view_factor: usize,

  /// Maximum size of a message, this applies to
  /// control and payload messages
  pub max_transmit_size: usize,

  /// How often a peer shuffle happens
  /// with a random active peer
  pub shuffle_interval: Duration,

  /// How long to keep pushing the IHAVE messages
  /// to lazy push peers in the plumtree.
  pub lazy_push_interval: Duration,

  /// How long IHAVE message ids are kept in history
  /// for identifying duplicate and missing messages.
  pub history_window: Duration,

  /// How often we send out IHaves and check for missing
  /// messages that other peers know about.
  pub tick_frequency: Duration,

  /// The difference in hops between observed IHAVEs and received
  /// messages that triggers tree optimization and replacing the
  /// eager node
  pub hop_optimization_factor: usize,

  /// A predicate that decides whether a given peer is allowed to
  /// join a topic. The default authenticator allows any peer to
  /// join any topic.
  ///
  /// There exist however cases when peers need to be authenticated
  /// before they can join the p2p topic, like for example having
  /// its identity stored in the list of validators or banning
  /// certain IP addresses.
  ///
  /// IMPORTANT: This predicate must always deterministically return
  /// the same result on all peers in a topic. If some peers allow a
  /// peer and others dont, then the whole network will misbehave because
  /// peers will start banning each other for protocol violation.
  ///
  /// This function must be really fast because it is invoked
  /// on every message from any peer.
  pub authorizer: PeerAuthorizer,
}

impl Config {
  pub fn max_active_view_size(&self) -> usize {
    ((self.network_size as f64).log2() + self.active_view_factor as f64).round()
      as usize
  }

  pub fn max_passive_view_size(&self) -> usize {
    self.max_active_view_size() * self.passive_view_factor
  }

  pub fn active_walk_length(&self) -> usize {
    ((self.network_size as f64).log2() as usize).clamp(2, 6)
  }

  pub fn shuffle_max_size(&self) -> usize {
    self.max_active_view_size() * 2
  }
}

impl Default for Config {
  /// Defaults inspired by the HyParView paper for a topic
  /// with 10 000 nodes participating in it.
  fn default() -> Self {
    Self {
      network_size: 1000,
      active_view_factor: 1,
      passive_view_factor: 6,
      max_transmit_size: 1_024_000, // 1 MB
      shuffle_interval: Duration::from_secs(60),
      lazy_push_interval: Duration::from_secs(2),
      history_window: Duration::from_secs(30),
      tick_frequency: Duration::from_millis(200),
      hop_optimization_factor: 4,
      authorizer: PeerAuthorizer::new(|_: &str, _: &PeerId| {
        true // allow all by default
      }),
    }
  }
}
