use {
  super::{
    config::Config,
    error::PublishError,
    handler::EpisubHandler,
    rpc,
    topic::TopicMesh,
    view::AddressablePeer,
  },
  futures::FutureExt,
  libp2p::{
    core::{
      connection::{ConnectionId, ListenerId},
      ConnectedPoint,
      Multiaddr,
      PeerId,
    },
    multiaddr::Protocol,
    swarm::{
      CloseConnection,
      DialError,
      NetworkBehaviour,
      NetworkBehaviourAction,
      NotifyHandler,
      PollParameters,
    },
  },
  rand::Rng,
  std::{
    collections::{HashMap, HashSet, VecDeque},
    iter,
    net::{Ipv4Addr, Ipv6Addr},
    task::{Context, Poll},
  },
  tracing::{debug, trace, warn},
};

/// Event that can be emitted by the episub behaviour.
#[derive(Debug)]
pub enum EpisubEvent {
  Message {
    topic: String,
    id: u128,
    payload: Vec<u8>,
  },
  Subscribed(String),
  PeerAdded(PeerId),
  PeerRemoved(PeerId),
  _Unsubscibed(String),
}

pub(crate) type EpisubNetworkBehaviourAction =
  NetworkBehaviourAction<EpisubEvent, EpisubHandler, rpc::Rpc>;

/// Network behaviour that handles the Episub protocol.
///
/// This network behaviour combines three academic papers into one
/// implementation:   1. HyParView: For topic-peer-membership management and
/// node discovery   2. Epidemic Broadcast Trees: For constructing efficient
/// broadcast trees and efficient content dessamination   3. GoCast:
/// Gossip-Enhanced Overlay Multicast for Fast and Dependable Group
/// Communication
pub struct Episub {
  /// Global behaviour configuration
  config: Config,

  /// Identity of this node
  local_node: Option<AddressablePeer>,

  /// Per-topic node membership
  topics: HashMap<String, TopicMesh>,

  /// A list of peers that have violated the protocol
  /// and are pemanently banned from this node. All their
  /// communication will be ignored and connections rejected.
  banned_peers: HashSet<PeerId>,

  /// Topics that we want to join, but haven't found a node
  /// to connect to.
  pending_topics: HashSet<String>,

  /// events that need to yielded to the outside when polling
  out_events: VecDeque<EpisubNetworkBehaviourAction>,

  /// a mapping of known peerid to the addresses they have dialed us from
  peer_addresses: HashMap<PeerId, Multiaddr>,

  /// This is the set of peers that we have managed to dial before we started
  /// listening on an external address that is not localhost. It does not make
  /// sense to send a JOIN message to peers without telling them where are we
  /// listening, so in cases when a connection is established, but we didn't
  /// get any valid address in local_node.addresses, we keep track of those
  /// peer id, and once we get a listen address, we send a join request to
  /// them.
  early_peers: HashSet<PeerId>,
}

impl Episub {
  pub fn new(config: Config) -> Self {
    Self {
      config,
      local_node: None,
      topics: HashMap::new(),
      peer_addresses: HashMap::new(),
      banned_peers: HashSet::new(),
      pending_topics: HashSet::new(),
      out_events: VecDeque::new(),
      early_peers: HashSet::new(),
    }
  }
}

impl Default for Episub {
  fn default() -> Self {
    Self::new(Config::default())
  }
}

impl Episub {
  /// Sends an rpc message to a connected peer.
  ///
  /// if the connection to the peer is dropped or otherwise the peer becomes
  /// unreachable, then this event is silently dropped.
  fn _send_message(&mut self, peer_id: PeerId, message: rpc::Rpc) {
    self
      .out_events
      .push_back(NetworkBehaviourAction::NotifyHandler {
        peer_id,
        event: message,
        handler: NotifyHandler::Any,
      })
  }
}

impl Episub {
  /// Subscribes to a gossip topic.
  ///
  /// Gossip topics are isolated clusters of nodes that gossip information, each
  /// topic maintins a separate HyParView of topic member nodes and does not
  /// interact with nodes from other topics (unless two nodes are subscribed
  /// to the same topic, but even then, they are not aware of the dual
  /// subscription).
  ///
  /// The subscription process is a long-living and dynamic process that has no
  /// end, and there is no point in time where we can decide that subscription
  /// is complete. What subscribing to a topic means, is that we will not
  /// ignore messages from peers that are sent for this topic id, this
  /// includes HyParView control messages or data messages.
  ///
  /// When subscribing to a new topic, we place the topic in the pending_joins
  /// collection that will send a join request to any node we connect to,
  /// until one of the nodes responds with another NEIGHBOR message.
  pub fn subscribe(&mut self, topic: String) -> bool {
    if self.topics.get(&topic).is_some() {
      debug!("Already subscribed to topic {}", topic);
      false
    } else {
      debug!("Subscribing to topic: {}", topic);
      if let Some(ref node) = self.local_node {
        self.topics.insert(
          topic.clone(),
          TopicMesh::new(topic.clone(), self.config.clone(), node.clone()),
        );
        self
          .out_events
          .push_back(EpisubNetworkBehaviourAction::GenerateEvent(
            EpisubEvent::Subscribed(topic),
          ));
      } else {
        self.pending_topics.insert(topic);
      }
      true
    }
  }

  /// Graceful removal from cluster.
  ///
  /// Stops responding to messages sent to this topic and informs
  /// all peers in the active and passive views that we are withdrawing
  /// from the cluster.
  pub fn _unsubscibe(&mut self, topic: String) -> bool {
    if self.topics.get(&topic).is_none() {
      warn!(
        "Attempt to unsubscribe from a non-subscribed topic {}",
        topic
      );
      false
    } else {
      debug!("unsubscribing from topic: {}", topic);
      self.pending_topics.remove(&topic);
      if let Some(mesh) = self.topics.remove(&topic) {
        for peer in mesh.nodes().active().map(|ap| ap.peer_id) {
          trace!("disconnecting from peer {} on topic {}", peer, topic);
          self._send_message(peer, rpc::Rpc {
            topic: topic.clone(),
            action: Some(rpc::rpc::Action::Disconnect(rpc::Disconnect {
              alive: false, // remove from peers passive view as well
            })),
          });
        }
      }

      self
        .out_events
        .push_back(EpisubNetworkBehaviourAction::GenerateEvent(
          EpisubEvent::_Unsubscibed(topic),
        ));

      true
    }
  }

  pub fn publish(
    &mut self,
    topic: &str,
    message: Vec<u8>,
  ) -> Result<u128, PublishError> {
    if let Some(topic) = self.topics.get_mut(topic) {
      let id = rand::thread_rng().gen();
      topic.publish(id, message);
      Ok(id)
    } else {
      Err(PublishError::TopicNotSubscribed)
    }
  }
}

impl NetworkBehaviour for Episub {
  type ConnectionHandler = EpisubHandler;
  type OutEvent = EpisubEvent;

  fn new_handler(&mut self) -> Self::ConnectionHandler {
    EpisubHandler::new(self.config.max_transmit_size, false)
  }

  fn inject_connection_established(
    &mut self,
    peer_id: &PeerId,
    connection: &ConnectionId,
    endpoint: &ConnectedPoint,
    _failed_addresses: Option<&Vec<Multiaddr>>,
    _other_established: usize,
  ) {
    if self.banned_peers.contains(peer_id) {
      self.force_disconnect(*peer_id, *connection);
      debug!(
        "Rejected connection from banned peer {} on endpoint {:?}",
        peer_id, endpoint
      );
      return;
    }

    debug!(
      "Connection to peer {} established on endpoint {:?}",
      peer_id, endpoint
    );

    // preserve a mapping from peer id to the address that was
    // used to establish the connection.
    self.peer_addresses.insert(*peer_id, match endpoint {
      ConnectedPoint::Dialer { address, .. } => address.clone(),
      ConnectedPoint::Listener { send_back_addr, .. } => send_back_addr.clone(),
    });

    // if this is us dialing a node, usually one of bootstrap nodes
    if matches!(endpoint, ConnectedPoint::Dialer { .. }) {
      // check if we are in the process of joining a topic, if so, for
      // each topic that has not found its cluster, send a join request
      // to every node that accepts a connection from us.

      // make sure that we know who we are and how we can be reached.
      if self.local_node.is_some() {
        // for each topic that has zero active nodes,
        // send a join request to any dialer
        self.request_join_for_starving_topics(*peer_id);
      } else {
        // otherwise keep note of this peer and send a join
        // request when we know who we are.
        self.early_peers.insert(*peer_id);
      }
    }
  }

  fn inject_new_listen_addr(&mut self, _id: ListenerId, addr: &Multiaddr) {
    // it does not make sense to advertise localhost addresses to remote nodes
    if !is_local_address(addr) {
      if let Some(node) = self.local_node.as_mut() {
        node.addresses.insert(addr.clone());
      }
    }
  }

  fn inject_expired_listen_addr(&mut self, _id: ListenerId, addr: &Multiaddr) {
    if let Some(node) = self.local_node.as_mut() {
      node.addresses.retain(|a| a != addr);
    }
  }

  fn inject_dial_failure(
    &mut self,
    peer_id: Option<PeerId>,
    _: Self::ConnectionHandler,
    error: &DialError,
  ) {
    if !matches!(error, DialError::DialPeerConditionFalse(_)) {
      if let Some(peer_id) = peer_id {
        debug!("Dialing peer {} failed: {:?}", peer_id, error);
        for (_, mesh) in self.topics.iter_mut() {
          // remove from active and passive
          mesh.disconnected(peer_id, false);
        }
      }
    }
  }

  fn inject_connection_closed(
    &mut self,
    peer_id: &PeerId,
    _: &ConnectionId,
    endpoint: &ConnectedPoint,
    _: EpisubHandler,
    _remaining_established: usize,
  ) {
    debug!(
      "Connection to peer {} closed on endpoint {:?}",
      peer_id, endpoint
    );

    for (_, mesh) in self.topics.iter_mut() {
      // remove from active keep in passive
      mesh.disconnected(*peer_id, true);
    }
  }

  /// Invoked on every message from the protocol for active connections with
  /// peers. Does basic validation only and forwards those calls to their
  /// appropriate handlers in HyParView or Plumtree
  fn inject_event(
    &mut self,
    peer_id: PeerId,
    connection: ConnectionId,
    event: rpc::Rpc,
  ) {
    if self.banned_peers.contains(&peer_id) {
      debug!(
        "rejecting event from a banned peer {}: {:?}",
        peer_id, event
      );
      self.force_disconnect(peer_id, connection);
      return;
    }

    if !self.config.authorizer.allow(&event.topic, &peer_id) {
      debug!(
        "disconnecting peer {} because it is not authorized on topic {}",
        &peer_id, &event.topic
      );
      self.force_disconnect(peer_id, connection);
      return;
    }

    trace!(
      "inject_event, peerid: {}, connection: {:?}, event: {:?}",
      peer_id,
      connection,
      event
    );

    if event.action.is_none() {
      self.ban_peer(peer_id, connection); // peer is violating the protocol
      return;
    }

    if let Some(mesh) = self.topics.get_mut(&event.topic) {
      // handle rpc call on an established topic, if the RPC message has
      // a syntax error or is unparsable at the protocol level, then ban
      // the sender from this node. This might indicate a malicious node
      // or an incompatible version of the protocol.
      if let Err(error) = mesh.inject_rpc_call(peer_id, event) {
        warn!("Protocol violation: {}", error);
        self.ban_peer(peer_id, connection);
      }
    } else {
      // reject any messages on a topic that we're not subscribed to
      // by immediately sending a disconnect event with alive: false,
      // then closing connection to the peer. That will also remove
      // this node from the requesting peer's passive view, so we won't
      // be bothered again by this peer for this topic.
      self
        .out_events
        .push_back(EpisubNetworkBehaviourAction::NotifyHandler {
          peer_id,
          handler: NotifyHandler::Any,
          event: rpc::Rpc {
            topic: event.topic,
            action: Some(rpc::rpc::Action::Disconnect(rpc::Disconnect {
              alive: false,
            })),
          },
        });
      self.force_disconnect(peer_id, connection);
    }
  }

  fn poll(
    &mut self,
    cx: &mut Context<'_>,
    params: &mut impl PollParameters,
  ) -> Poll<NetworkBehaviourAction<Self::OutEvent, Self::ConnectionHandler>> {
    // update local peer identity and addresses
    self.update_local_node_info(params);

    // bubble up any outstanding behaviour-level events in fifo order
    if let Some(event) = self.out_events.pop_front() {
      return Poll::Ready(event);
    }

    // next bubble up events for all topics
    // todo, randomize polling among topics, otherwise
    // some topics might be starved by more active ones
    for mesh in self.topics.values_mut() {
      if let Poll::Ready(event) = mesh.poll_unpin(cx) {
        return Poll::Ready(event);
      }
    }

    Poll::Pending
  }
}

impl Episub {
  /// updates our own information abount ourselves.
  /// This includes our own peer id, the addresses we are listening on
  /// and any external addresses we are aware of. Excludes any localhost
  /// addresses
  fn update_local_node_info(&mut self, params: &impl PollParameters) {
    if self.local_node.is_none() {
      let addresses: HashSet<_> = params
        .external_addresses()
        .map(|ad| ad.addr)
        .chain(params.listened_addresses())
        .filter(|a| !is_local_address(a))
        .collect();

      if !addresses.is_empty() {
        self.local_node.replace(AddressablePeer {
          peer_id: *params.local_peer_id(),
          addresses,
        });

        for topic in self.pending_topics.drain() {
          // for any subscripts requested before we
          // we knew about our own peer identity and
          // addresses
          if !self.topics.contains_key(&topic) {
            self.topics.insert(
              topic.clone(),
              TopicMesh::new(
                topic.clone(),
                self.config.clone(),
                self.local_node.as_ref().unwrap().clone(),
              ),
            );
          }
        }

        // request JOINS for peers that have been dialed before we started
        // listening and knew our identity.

        // because both references to self are mut,
        // this works around the borrow checker,
        // although the actual fields in self.early_peers
        // and the ones accessed by self.request_join...()
        // are disjoin, the compiler is seeing it as double
        // mut borrow to self.
        #[allow(clippy::needless_collect)]
        let early_peers: Vec<PeerId> = self.early_peers.drain().collect();
        early_peers
          .into_iter()
          .for_each(|p| self.request_join_for_starving_topics(p));
      }
    }
  }

  fn ban_peer(&mut self, peer: PeerId, connection: ConnectionId) {
    warn!("Banning peer {}", peer);
    self.peer_addresses.remove(&peer);
    self.banned_peers.insert(peer);
    self.force_disconnect(peer, connection);
  }

  fn force_disconnect(&mut self, peer: PeerId, connection: ConnectionId) {
    self
      .out_events
      .push_back(EpisubNetworkBehaviourAction::CloseConnection {
        peer_id: peer,
        connection: CloseConnection::One(connection),
      });
  }

  fn request_join_for_starving_topics(&mut self, peer: PeerId) {
    // for each topic that has zero active nodes,
    // send a join request to any dialer
    self
      .topics
      .iter_mut()
      .filter(|(t, v)| {
        v.nodes().starved()
          && !v.nodes().is_active(&peer)
          && self.config.authorizer.allow(t, &peer)
      })
      .for_each(|(_, v)| {
        v.initiate_join(AddressablePeer {
          peer_id: peer,
          addresses: iter::once(self.peer_addresses.get(&peer).unwrap())
            .cloned()
            .collect(),
        })
      });
  }
}

/// This handles the case when the swarm api starts listening on
/// 0.0.0.0 and one of the addresses is localhost. Localhost is
/// meaningless when advertised to remote nodes, so its omitted
/// when counting local addresses
fn is_local_address(addr: &Multiaddr) -> bool {
  addr.iter().any(|p| {
    // fileter out all localhost addresses
    if let Protocol::Ip4(addr) = p {
      addr == Ipv4Addr::LOCALHOST || addr == Ipv4Addr::UNSPECIFIED
    } else if let Protocol::Ip6(addr) = p {
      addr == Ipv6Addr::LOCALHOST || addr == Ipv6Addr::UNSPECIFIED
    } else {
      false
    }
  })
}
