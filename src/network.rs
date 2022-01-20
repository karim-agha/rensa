use crate::{
  consensus::{
    block::{self, Block},
    vote::Vote,
  },
  keys::{Keypair, Pubkey},
};
use flexbuffers::FlexbufferSerializer;
use futures::StreamExt;
use libp2p::{
  core::{muxing::StreamMuxerBox, transport::Boxed, upgrade::Version},
  dns::{DnsConfig, ResolverConfig, ResolverOpts},
  identity::{self, ed25519::SecretKey},
  noise,
  swarm::{DialError, SwarmEvent},
  tcp::TcpConfig,
  yamux::YamuxConfig,
  Multiaddr, PeerId, Swarm, Transport,
};
use libp2p_episub::{Config, Episub, EpisubEvent, PeerAuthorizer};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, io::ErrorKind, marker::PhantomData};
use tracing::error;

type BoxedTransport = Boxed<(PeerId, StreamMuxerBox)>;

async fn create_transport(
  keypair: &Keypair,
) -> std::io::Result<BoxedTransport> {
  let transport = {
    let tcp = TcpConfig::new().nodelay(true).port_reuse(true);
    let dns_tcp = DnsConfig::custom(
      tcp,
      ResolverConfig::default(),
      ResolverOpts::default(),
    )
    .await?;
    dns_tcp
  };

  let noise_keys = noise::Keypair::<noise::X25519Spec>::new()
    .into_authentic(&identity::Keypair::Ed25519(
      SecretKey::from_bytes(keypair.secret().to_bytes())
        .unwrap()
        .into(),
    ))
    .expect("Signing libp2p-noise static DH keypair failed.");

  Ok(
    transport
      .upgrade(Version::V1)
      .authenticate(noise::NoiseConfig::xx(noise_keys).into_authenticated())
      .multiplex(YamuxConfig::default())
      .boxed(),
  )
}

// this is a bug in clippy, I filed an issue on GH:
// https://github.com/rust-lang/rust-clippy/issues/8321
// remove this when the issue gets closed.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum NetworkEvent<D>
where
  D: Eq + Serialize + for<'a> Deserialize<'a>,
{
  BlockReceived(block::Produced<D>),
  VoteReceived(Vote),
}

pub struct Network<D>
where
  D: Eq + Serialize + for<'a> Deserialize<'a>,
{
  swarm: Swarm<Episub>,
  chainid: String,
  _marker: PhantomData<D>,
}

impl<D> Network<D>
where
  D: Eq + Serialize + for<'a> Deserialize<'a>,
{
  pub async fn new(
    genesis: &block::Genesis<D>,
    keypair: Keypair,
    listenaddrs: impl Iterator<Item = Multiaddr>,
  ) -> std::io::Result<Self> {
    let id = identity::Keypair::Ed25519(
      identity::ed25519::SecretKey::from_bytes(
        &mut keypair.secret().to_bytes(),
      )
      .unwrap()
      .into(),
    );

    // allow only validators to join this p2p network.
    // dynamic validator membership is not implemented in this
    // iteration of the consensus algorithm.

    // build an O(1) quick lookup structure for validators
    let vset: HashSet<_> = genesis
      .validators
      .iter()
      .map(|v| v.pubkey.clone())
      .collect();

    // use an authentiator predicate that denies connections
    // to any peer id that is not a known validator.
    let authorizer = PeerAuthorizer::new(move |_, peerid| {
      let pubkey: Pubkey = (*peerid).into();
      vset.contains(&pubkey)
    });

    let mut swarm = Swarm::new(
      create_transport(&keypair).await?,
      Episub::new(Config {
        authorizer,
        network_size: genesis.validators.len(),
        ..Config::default()
      }),
      id.public().to_peer_id(),
    );

    listenaddrs.for_each(|addr| {
      swarm.listen_on(addr).unwrap();
    });

    let chainid = genesis.chain_id.clone();

    swarm
      .behaviour_mut()
      .subscribe(format!("/{}/vote", &chainid));
    swarm
      .behaviour_mut()
      .subscribe(format!("/{}/blocks", &chainid));
    swarm
      .behaviour_mut()
      .subscribe(format!("/{}/txs", &chainid));

    Ok(Self {
      swarm,
      chainid,
      _marker: PhantomData,
    })
  }

  pub fn connect(&mut self, addr: Multiaddr) -> Result<(), DialError> {
    self.swarm.dial(addr)
  }

  pub fn gossip_vote(&mut self, vote: &Vote) -> Result<u128, std::io::Error> {
    self.gossip_generic(&format!("/{}/vote", self.chainid), vote)
  }

  pub fn gossip_block(
    &mut self,
    block: &impl Block<D>,
  ) -> Result<u128, std::io::Error>
  where
    D: Serialize + Eq + for<'a> Deserialize<'a>,
  {
    self.gossip_generic(&format!("/{}/blocks", self.chainid), block)
  }

  fn gossip_generic(
    &mut self,
    topic: &str,
    data: &impl Serialize,
  ) -> Result<u128, std::io::Error> {
    let mut s = FlexbufferSerializer::new();
    data
      .serialize(&mut s)
      .map_err(|e| std::io::Error::new(ErrorKind::InvalidInput, e))?;

    self
      .swarm
      .behaviour_mut()
      .publish(topic, s.take_buffer())
      .map_err(|e| std::io::Error::new(ErrorKind::InvalidInput, e))
  }

  pub async fn next(&mut self) -> Option<NetworkEvent<D>>
  where
    D: Serialize + Eq + for<'a> Deserialize<'a>,
  {
    loop {
      if let Some(SwarmEvent::Behaviour(EpisubEvent::Message {
        topic,
        payload,
        ..
      })) = self.swarm.next().await
      {
        if topic == format!("/{}/vote", self.chainid) {
          match flexbuffers::from_slice(&payload) {
            Ok(vote) => return Some(NetworkEvent::VoteReceived(vote)),
            Err(e) => error!("Failed to deserialize vote: {e}"),
          }
        } else if topic == format!("/{}/blocks", self.chainid) {
          match flexbuffers::from_slice(&payload) {
            Ok(block) => return Some(NetworkEvent::BlockReceived(block)),
            Err(e) => error!("Failed to deserialize block: {e}"),
          }
        }
      }
    }
  }
}
