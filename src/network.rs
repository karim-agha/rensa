use crate::{
  consensus::validator::Validator,
  keys::{Keypair, Pubkey},
};
use futures::StreamExt;
use libp2p::{
  core::{muxing::StreamMuxerBox, transport::Boxed, upgrade::Version},
  dns::{DnsConfig, ResolverConfig, ResolverOpts},
  identity::{self, ed25519::SecretKey},
  noise,
  swarm::{DialError, NetworkBehaviour, ProtocolsHandler, SwarmEvent},
  tcp::TcpConfig,
  yamux::YamuxConfig,
  Multiaddr, PeerId, Swarm, Transport,
};
use libp2p_episub::{Config, Episub, EpisubEvent, PeerAuthorizer};
use std::collections::HashSet;

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

pub struct Network {
  swarm: Swarm<Episub>,
  topic: String,
}

type EpisubProtocolHandler = <Episub as NetworkBehaviour>::ProtocolsHandler;
type EpisubProtocolError = <EpisubProtocolHandler as ProtocolsHandler>::Error;

impl Network {
  pub async fn new(
    chainid: impl AsRef<str>,
    validators: &[Validator],
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

    // allow only known validators to join this p2p network.
    // dynamic validator membership is not implemented in this
    // iteration of the consensus algorithm.

    // build an O(1) quick lookup structure for validators
    let vset: HashSet<_> =
      validators.iter().map(|v| v.pubkey.clone()).collect();

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
        network_size: validators.len(),
        ..Config::default()
      }),
      id.public().to_peer_id(),
    );

    listenaddrs.for_each(|addr| {
      swarm.listen_on(addr).unwrap();
    });

    let topic = format!("/{}/gossip", chainid.as_ref());
    swarm.behaviour_mut().subscribe(topic.clone());

    Ok(Self { swarm, topic })
  }

  pub fn connect(&mut self, addr: Multiaddr) -> Result<(), DialError> {
    self.swarm.dial(addr)
  }

  pub fn gossip(
    &mut self,
    data: Vec<u8>,
  ) -> Result<u128, impl std::error::Error> {
    self.swarm.behaviour_mut().publish(&self.topic, data)
  }

  pub async fn next(
    &mut self,
  ) -> Option<SwarmEvent<EpisubEvent, EpisubProtocolError>> {
    self.swarm.next().await
  }
}
