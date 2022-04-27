//! Plumtree: Epidemic Broadcast Trees
//! Leitão, João & Pereira, José & Rodrigues, Luís. (2007).
//! 301-310. 10.1109/SRDS.2007.27.

use {
  super::{
    behaviour::EpisubNetworkBehaviourAction,
    cache::{ExpiringCache, Keyed, MessageInfo, MessageRecord},
    rpc,
    Config,
    EpisubEvent,
  },
  asynchronous_codec::Bytes,
  libp2p::{core::PeerId, swarm::NotifyHandler},
  std::{
    collections::{hash_map::Entry, HashMap, HashSet, VecDeque},
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    time::Instant,
  },
  tracing::{debug, error},
  zstd::decode_all,
};

pub struct PlumTree {
  topic: String,
  local_node: PeerId,
  lazy: HashSet<PeerId>,
  eager: HashSet<PeerId>,
  last_tick: Instant,
  config: Config,
  observed: ExpiringCache<MessageInfo>,
  received: ExpiringCache<MessageRecord>,
  out_events: VecDeque<EpisubNetworkBehaviourAction>,
}

impl PlumTree {
  pub fn new(topic: String, config: Config, local_node: PeerId) -> Self {
    PlumTree {
      topic,
      config,
      local_node,
      lazy: HashSet::new(),
      eager: HashSet::new(),
      last_tick: Instant::now(),
      observed: ExpiringCache::new(),
      received: ExpiringCache::new(),
      out_events: VecDeque::new(),
    }
  }

  /// Called when the peer sampling service (HyparView) activates a peer
  pub fn inject_neighbor_up(&mut self, peer: PeerId) {
    self.eager.insert(peer);
  }

  /// Called when the peer sampling service (HyparView) deactivates a peer
  pub fn inject_neighbor_down(&mut self, peer: PeerId) {
    self.eager.remove(&peer);
    self.lazy.remove(&peer);
  }

  pub fn publish(&mut self, id: u64, payload: Bytes) {
    if let Some(msg) = self.received.get(&id) {
      error!(
        "not publishing a message with id {}, received previously from node {}",
        id, msg.sender
      );
      return;
    }

    if let Some(msg) = self.observed.get(&id) {
      error!(
        "refusing to send a message with id {}, observed previously by node {}",
        id, msg.sender
      );
      return;
    }

    let message = MessageRecord {
      id,
      payload,
      hop: 1,
      sender: self.local_node,
    };

    for enode in &self.eager {
      debug!("sending message {} to peer {}", id, enode);
      self
        .out_events
        .push_back(EpisubNetworkBehaviourAction::NotifyHandler {
          peer_id: *enode,
          handler: NotifyHandler::Any,
          event: rpc::Rpc {
            topic: self.topic.clone(),
            action: Some(rpc::rpc::Action::Message(message.clone().into())),
          },
        });
    }

    // mark this message as received, so if we get it
    // again from other nodes we know that there is a
    // cycle in the broadcast tree. Also those messages are
    // published as IHAVEs and replayed on demand when grafting
    // links with lazy nodes.
    self.received.insert(MessageRecord { hop: 0, ..message });
  }

  pub fn inject_message(
    &mut self,
    peer_id: PeerId,
    id: u64,
    hop: u32,
    payload: Bytes,
  ) {
    debug!(
      "received message from {} with id {} [hop {}]",
      peer_id, id, hop
    );

    // if we don't have this message in the message cache
    // it means that we're seeing it for the first time,
    // then forward it to all eager push nodes.
    if self.received.insert(MessageRecord {
      id,
      hop,
      payload: payload.clone(),
      sender: peer_id,
    }) {
      let out = match self.config.enable_compression {
        true => Bytes::from(decode_all(payload.as_ref()).unwrap()),
        false => payload.clone(),
      };
      self
        .out_events
        .push_back(EpisubNetworkBehaviourAction::GenerateEvent(
          EpisubEvent::Message {
            topic: self.topic.clone(),
            id,
            payload: out,
          },
        ));

      let message = rpc::Rpc {
        topic: self.topic.clone(),
        action: Some(rpc::rpc::Action::Message(rpc::Message {
          payload,
          id,
          hop: hop + 1,
        })),
      };

      // push message to all eager peers, except the sender
      for peer in &self.eager {
        if peer == &peer_id {
          continue;
        }
        self.out_events.push_back(
          EpisubNetworkBehaviourAction::NotifyHandler {
            peer_id: *peer,
            handler: NotifyHandler::Any,
            event: message.clone(),
          },
        );
        debug!("sending message {} to peer {}", id, peer);
      }
    } else if self.config.optimize_sender_tree {
      // this is a duplicate message, it means that we are
      // having a cycle in the node connectivity graph. The
      // sender should be moved to lazy push peers and notified
      // that we have moved them to lazy nodes.
      if self.eager.contains(&peer_id) {
        self.inject_prune(peer_id);
        self.out_events.push_back(
          EpisubNetworkBehaviourAction::NotifyHandler {
            peer_id,
            handler: NotifyHandler::Any,
            event: rpc::Rpc {
              topic: self.topic.clone(),
              action: Some(rpc::rpc::Action::Prune(rpc::Prune {})),
            },
          },
        );
        debug!("pruning link with {}", peer_id);
      }
    }
  }

  pub fn inject_ihave(&mut self, peer_id: PeerId, id: u64, hop: u32) {
    self.observed.insert(MessageInfo {
      id,
      hop,
      sender: peer_id,
    });
  }

  pub fn inject_prune(&mut self, peer_id: PeerId) {
    self.eager.remove(&peer_id);
    self.lazy.insert(peer_id);
  }

  pub fn inject_graft(&mut self, peer_id: PeerId, ids: Vec<u64>) {
    // upgrade to eager node after graft
    self.lazy.remove(&peer_id);
    self.eager.insert(peer_id);

    // and send all missing messages
    ids
      .into_iter()
      .filter_map(|id| self.received.get(&id))
      .for_each(|msg| {
        self
          .out_events
          .push_back(EpisubNetworkBehaviourAction::NotifyHandler {
            peer_id,
            handler: NotifyHandler::Any,
            event: rpc::Rpc {
              topic: self.topic.clone(),
              action: Some(rpc::rpc::Action::Message(msg.clone().into())),
            },
          })
      });
  }
}

impl PlumTree {
  /// send IHAVEs to all lazy push nodes
  fn publish_ihaves(&mut self) {
    let time_range_begin = Instant::now() - self.config.lazy_push_window;
    let time_range_end = Instant::now();
    let received: Vec<_> = self
      .received
      .iter_range(time_range_begin..time_range_end)
      .map(|m| rpc::i_have::MessageRecord {
        id: m.id,
        hop: m.hop,
      })
      .collect();

    if !received.is_empty() {
      self.lazy.iter().for_each(|p| {
        self
          .out_events
          .push_back(EpisubNetworkBehaviourAction::NotifyHandler {
            peer_id: *p,
            handler: NotifyHandler::Any,
            event: rpc::Rpc {
              topic: self.topic.clone(),
              action: Some(rpc::rpc::Action::Ihave(rpc::IHave {
                ihaves: received.clone(),
              })),
            },
          })
      });
    }
  }

  fn prune_history(&mut self) {
    let prune_cutoff = Instant::now() - self.config.history_window;

    // remove old entries from history
    self.observed.remove_older_than(prune_cutoff);
    self.received.remove_older_than(prune_cutoff);
  }

  /// triggered every configured tick Duration, it attemtps
  /// to find if we are missing any messages that were not delivered
  /// to this node by eager nodes, or the there is a much more optimal
  /// path for messages from one of the lazy push nodes.
  fn repair_tree(&mut self) {
    {
      // check for messages that we were told about by lazy push peers
      // and check if they are in the received messages. If not then it
      // means that we have missing messages, and then we will need to
      // graft the connection to the peer that told us about the missing
      // message. Also we replace the eager push link if the hop count
      // in the observed messages is significantly lower than what we
      // received from the eager push nodes.
      let time_range_begin = Instant::now() - self.config.lazy_push_window;
      let time_range_end = Instant::now() - 2 * self.config.tick_frequency;
      let expected_ihaves: HashSet<_> = self
        .observed
        .iter_range(time_range_begin..time_range_end)
        .collect();

      let mut grafts = HashMap::<PeerId, Vec<u64>>::new();
      for observed in expected_ihaves {
        match self.received.get(&observed.key()) {
          Some(received) => {
            if received.hop.saturating_sub(observed.hop)
              >= self.config.hop_optimization_factor
              && observed.hop != 0
            {
              debug!(
                "path for message {} from {} is better than {} ({}:{})",
                received.id,
                observed.sender,
                received.sender,
                received.hop,
                observed.hop
              );

              // we have the message, so no need to request it again,
              // but the path it took to reach us is much shorter, so
              // just graft the connection but don't ask for the message.
              if let Entry::Vacant(v) = grafts.entry(observed.sender) {
                // this will make this sender an active push node again,
                // if the path indeed is faster than the old one, then
                // this will remain eager push and the old one will be
                // pruned, because the message will arrive from the new
                // eager node first, otherwise, if it was just a temporary
                // network instability, the tree structure will go back to
                // its previous state.
                v.insert(vec![]);
              }
            }
          }
          None => {
            debug!(
              "message {} was NOT received, but observed by peer {}",
              observed.id, observed.sender
            );
            // we have a missing message, so we need to request it from
            // the node we are grafting our connection to.
            match grafts.entry(observed.sender) {
              Entry::Vacant(v) => {
                v.insert(vec![observed.id]);
              }
              Entry::Occupied(mut o) => {
                o.get_mut().push(observed.id);
              }
            }
          }
        }
      }

      grafts.into_iter().for_each(|(p, ids)| {
        debug!("grafting link with {p}: [{ids:?}]");
        self
          .out_events
          .push_back(EpisubNetworkBehaviourAction::NotifyHandler {
            peer_id: p,
            handler: NotifyHandler::Any,
            event: rpc::Rpc {
              topic: self.topic.clone(),
              action: Some(rpc::rpc::Action::Graft(rpc::Graft { ids })),
            },
          })
      });
    }

    self.prune_history();
  }
}

impl Future for PlumTree {
  type Output = EpisubNetworkBehaviourAction;

  fn poll(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
    // start with periodic tree maintenance and
    // batch IHAVE advertisements to lazy nodes
    if Instant::now().duration_since(self.last_tick)
      > self.config.tick_frequency
    {
      self.publish_ihaves();
      self.repair_tree();
      self.last_tick = Instant::now();
    }

    if let Some(event) = self.out_events.pop_front() {
      return Poll::Ready(event);
    }

    Poll::Pending
  }
}
