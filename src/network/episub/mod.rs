//! Episub: Proximity Aware Epidemic PubSub for libp2p
//!
//! This behaviour implements a large-scale gossiping protocol that is based on
//! two main ideas introduced by the following papers:
//!
//!   1. Epidemic Broadcast Trees, 2007 (DOI: 10.1109/SRDS.2007.27)
//!   2. HyParView: a membership protocol for reliable gossip-based broadcast,
//!      2007 (DOI: 10.1109/DSN.2007.56)
//!
//! Those ideas were first compiled into one protocol originally by @vyzo in
//! https://github.com/libp2p/specs/blob/master/pubsub/gossipsub/episub.md
//!
//! This implementation introduces a number of small changes to the original
//! proposal that surfaced during implementation and testing of this code.
//!
//! # Usage Examples
//!
//! ```
//! let local_key = identity::Keypair::generate_ed25519();
//! let local_peer_id = PeerId::from(local_key.public());
//! let transport = libp2p::development_transport(local_key.clone()).await?;
//!
//! // Create a Swarm to manage peers and events
//! let mut swarm = libp2p::Swarm::new(transport, Episub::new(), local_peer_id);
//!
//! // Listen on all interfaces and whatever port the OS assigns
//! swarm
//!   .listen_on("/ip4/0.0.0.0/tcp/4001".parse().unwrap())
//!   .unwrap();
//!
//! // subscribe to the topic specified on the command line
//! swarm.behaviour_mut().subscribe(opts.topic);
//! swarm.dial(bootstrap).unwrap()
//!
//! while let Some(event) = swarm.next().await {
//!   match event {
//!      SwarmEvent::Behaviour(EpisubEvent::Message(m, t)) => {
//!         println!("got a message: {:?} on topic {}", m, t);
//!      }
//!      SwarmEvent::Behaviour(EpisubEvent::Subscribed(t)) => {}
//!      SwarmEvent::Behaviour(EpisubEvent::Unsubscribed(t)) => {}
//!      SwarmEvent::Behaviour(EpisubEvent::ActivePeerAdded(p)) => {}
//!      SwarmEvent::Behaviour(EpisubEvent::ActivePeerRemoved(p)) => {}
//!   }
//! }
//! ```

#[allow(clippy::module_inception)]

mod rpc {
  include!(concat!(env!("OUT_DIR"), "/rpc.pb.rs"));
}

mod behaviour;
mod cache;
mod codec;
mod config;
mod connection;
mod error;
mod handler;
mod topic;
mod tree;
mod view;

pub use {
  behaviour::{Episub, EpisubEvent},
  config::{Config, PeerAuthorizer},
  error::{EpisubHandlerError, FormatError, PublishError, RpcError},
};
