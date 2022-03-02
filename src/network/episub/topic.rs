use {
  super::{
    behaviour::EpisubNetworkBehaviourAction,
    config::Config,
    error::RpcError,
    rpc::{self, rpc::Action},
    tree::PlumTree,
    view::{AddressablePeer, HyParView},
    EpisubEvent,
  },
  asynchronous_codec::Bytes,
  futures::FutureExt,
  libp2p::{core::PeerId, swarm::NetworkBehaviourAction},
  std::{
    collections::VecDeque,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
  },
  tracing::{debug, warn},
};

/// Represents a view of the network from one topic's perspective.
///
/// Each topic has its own HyParView instance that form their own cluster of
/// nodes for message dissemination
pub struct TopicMesh {
  tree: PlumTree,
  nodes: HyParView,
  local_node: AddressablePeer,
  out_events: VecDeque<EpisubNetworkBehaviourAction>,
}

impl TopicMesh {
  pub fn new(topic: String, config: Config, local: AddressablePeer) -> Self {
    TopicMesh {
      tree: PlumTree::new(topic.clone(), config.clone(), local.peer_id),
      local_node: local.clone(),
      nodes: HyParView::new(topic, config, local),
      out_events: VecDeque::new(),
    }
  }

  /// Access the underlying local HyParView that manages
  /// connections with active and passive nodes of the mesh.
  pub fn nodes(&self) -> &HyParView {
    &self.nodes
  }

  /// Invoked when a connection to a peer is lost or closed.
  /// If the connection closed for unrecoverable reasons (like
  /// undialbale address) then alive is false, and that causes the
  /// peer to be also removed from the passive view. Otherwise the
  /// peer is only removed from the active view and kept in passive.
  ///
  /// calling this method has the same effect as if the peer sent
  /// a DISCONNECT message with ALIVE = false.
  pub fn disconnected(&mut self, peer_id: PeerId, alive: bool) {
    self.nodes.inject_disconnect(peer_id, alive);
  }

  pub fn initiate_join(&mut self, peer: AddressablePeer) {
    self.nodes.initiate_join(peer);
  }

  pub fn publish(&mut self, id: u64, payload: Bytes) {
    debug!(
      "publishing message id {} with payload len {}",
      id,
      payload.len()
    );
    self.tree.publish(id, payload);
  }

  /// Routes RPC calls to HyParView and MessageGraph from active nodes.
  /// Returns true if the message passes basic protocol validation and was
  /// ingested, otherwise returns false that the message
  pub fn inject_rpc_call(
    &mut self,
    peer_id: PeerId,
    event: rpc::Rpc,
  ) -> Result<(), RpcError> {
    match event.action.unwrap() {
      Action::Join(rpc::Join { ttl, peer }) => {
        if let Ok(peer) = peer.try_into() {
          self.nodes.inject_join(peer_id, peer, ttl);
        } else {
          return Err(RpcError::InvalidPeerId);
        }
      }
      Action::ForwardJoin(rpc::ForwardJoin { ttl, peer }) => {
        if let Ok(peer) = peer.try_into() {
          self.nodes.inject_forward_join(
            peer,
            ttl as usize,
            self.local_node.peer_id,
            peer_id,
          );
        } else {
          return Err(RpcError::InvalidPeerId);
        }
      }
      Action::Neighbor(rpc::Neighbor { priority, peer }) => {
        if let Ok(peer) = TryInto::<AddressablePeer>::try_into(peer) {
          if peer.peer_id == peer_id {
            self.nodes.inject_neighbor(peer, priority);
          } else {
            warn!("peer {} is impersonating {}", peer_id, peer.peer_id);
            return Err(RpcError::ImpersonatedPeer(peer_id, peer.peer_id));
          }
        } else {
          return Err(RpcError::InvalidPeerId);
        }
      }
      Action::Disconnect(rpc::Disconnect { alive }) => {
        self.nodes.inject_disconnect(peer_id, alive);
      }
      Action::Shuffle(rpc::Shuffle { origin, nodes, ttl }) => {
        if let Ok(origin) = origin.try_into() {
          self.nodes.inject_shuffle(
            peer_id,
            ttl,
            nodes
              .into_iter()
              .filter_map(|n| n.try_into().ok())
              .collect(),
            origin,
          );
        } else {
          return Err(RpcError::InvalidPeerId);
        }
      }
      Action::ShuffleReply(params) => {
        self.nodes.inject_shuffle_reply(params);
      }
      Action::Message(rpc::Message { id, hop, payload }) => {
        self.tree.inject_message(peer_id, id, hop, payload);
      }
      Action::Ihave(rpc::IHave { ihaves }) => {
        ihaves
          .iter()
          .map(|ih| (ih.id, ih.hop))
          .for_each(|(id, hop)| self.tree.inject_ihave(peer_id, id, hop));
      }
      Action::Prune(rpc::Prune { .. }) => {
        self.tree.inject_prune(peer_id);
      }

      Action::Graft(rpc::Graft { ids }) => {
        self.tree.inject_graft(peer_id, ids);
      }
    };

    Ok(())
  }
}

impl Future for TopicMesh {
  type Output = EpisubNetworkBehaviourAction;

  fn poll(
    mut self: Pin<&mut Self>,
    cx: &mut Context<'_>,
  ) -> Poll<Self::Output> {
    // first buuble up mesh events
    if let Some(event) = self.out_events.pop_front() {
      return Poll::Ready(event);
    }

    // then bubble up all HyParView events
    if let Poll::Ready(event) = self.nodes.poll_unpin(cx) {
      // also notify the Plumtree about active view changes so it
      // can construct it can construct and repair its tree.
      match &event {
        NetworkBehaviourAction::GenerateEvent(EpisubEvent::PeerAdded(peer)) => {
          self.tree.inject_neighbor_up(*peer);
        }
        NetworkBehaviourAction::GenerateEvent(EpisubEvent::PeerRemoved(
          peer,
        )) => {
          self.tree.inject_neighbor_down(*peer);
        }
        _ => {}
      }

      return Poll::Ready(event);
    }

    // the bubble up all PlumTree events
    if let Poll::Ready(event) = self.tree.poll_unpin(cx) {
      return Poll::Ready(event);
    }

    Poll::Pending
  }
}
