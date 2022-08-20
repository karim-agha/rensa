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

  /// If it has come time to perform shuffle, this
  /// specifies the probability that a shuffle will
  /// actually ocurr. Valid values are 0.0 - 1.0.
  ///
  /// This parameter is used in cases when a network
  /// peers don't all shuffle at the same time if they
  /// have the same [`shuffle_interval`] specified.
  ///
  /// Shuffle from other peers will populate the passive
  /// view anyway.
  pub shuffle_probability: f32,

  /// How long to keep pushing the IHAVE messages
  /// to lazy push peers in the plumtree.
  pub lazy_push_window: Duration,

  /// How long IHAVE message ids are kept in history
  /// for identifying duplicate and missing messages.
  pub history_window: Duration,

  /// How often we send out IHaves and check for missing
  /// messages that other peers know about.
  pub tick_frequency: Duration,

  /// Enables or disables the creation of a minimum spanning
  /// tree among peers. Set this to true if the sender doesn't
  /// change often, in that case the propagation gets more efficient
  /// as graph cycles are pruned and a minimum spanning tree is formed,
  /// otherwise, if the publishing node changes often then this
  /// causes excessive churn.
  pub optimize_sender_tree: bool,

  /// The difference in hops between observed IHAVEs and received
  /// messages that triggers tree optimization and replacing the
  /// eager node
  pub hop_optimization_factor: u32,

  /// Defines if message payloads are going to be compressed over the wire.
  /// This trades processing speed vs network bandwidth. Both sides of the
  /// p2p protocol must use the same configuration, otherwise they will
  /// attempt to decompress uncompressed data.
  pub enable_compression: bool,

  /// A value between 1 and 21. Value of 0 uses zstd default compression
  /// factor. See https://github.com/facebook/zstd for more info.
  pub compression_level: i32,

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

  /// A node is considered starving when it's active view size is less than 
  /// this value. It will try to maintain half of `max_active_view_size` to 
  /// achieve minimum level of connection redundancy, another half is reserved 
  /// for peering connections from other nodes.
  /// 
  /// Two thresholds allow to avoid cyclical connections and disconnections when new
  /// nodes are connected to a group of overconnected nodes.
  pub fn min_active_view_size(&self) -> usize {
    self.max_active_view_size().div_euclid(2).max(1)
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
      enable_compression: true,
      compression_level: 0,
      shuffle_probability: 1.0,     // always shuffle
      max_transmit_size: 1_024_000, // 1 MB
      shuffle_interval: Duration::from_secs(60),
      lazy_push_window: Duration::from_secs(2),
      history_window: Duration::from_secs(30),
      tick_frequency: Duration::from_millis(200),
      hop_optimization_factor: 4,
      optimize_sender_tree: true,
      authorizer: PeerAuthorizer::new(|_: &str, _: &PeerId| {
        true // allow all by default
      }),
    }
  }
}
