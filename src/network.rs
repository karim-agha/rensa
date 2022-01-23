use crate::{
  consensus::{
    block::{self, Block, BlockData},
    vote::Vote,
  },
  keys::{Keypair, Pubkey},
};
use futures::{Stream, StreamExt};
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
use std::{
  collections::HashSet,
  io::ErrorKind,
  marker::PhantomData,
  pin::Pin,
  task::{Context, Poll},
};
use tracing::{debug, error, warn};

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
pub enum NetworkEvent<D: BlockData> {
  BlockReceived(block::Produced<D>),
  VoteReceived(Vote),
}

pub struct Network<D: BlockData> {
  swarm: Swarm<Episub>,
  chainid: String,
  _marker: PhantomData<D>,
}

impl<D: BlockData> Network<D> {
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
        // 2 epochs are needed until block finalization
        history_window: genesis.slot_interval
          * (genesis.epoch_slots as u32 * 2),
        // keep informing all peers about all messages received for the last epoch
        lazy_push_interval: genesis.slot_interval * genesis.epoch_slots as u32,
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
      .subscribe(format!("/{}/block", &chainid));
    swarm.behaviour_mut().subscribe(format!("/{}/tx", &chainid));

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
    self
      .swarm
      .behaviour_mut()
      .publish(&format!("/{}/vote", self.chainid), vote.to_bytes()?)
      .map_err(|e| std::io::Error::new(ErrorKind::InvalidInput, e))
  }

  pub fn gossip_block(
    &mut self,
    block: &impl Block<D>,
  ) -> Result<u128, std::io::Error> {
    self
      .swarm
      .behaviour_mut()
      .publish(&format!("/{}/block", self.chainid), block.to_bytes()?)
      .map_err(|e| std::io::Error::new(ErrorKind::InvalidInput, e))
  }
}

impl<D: BlockData> Unpin for Network<D> {}
impl<D: BlockData> Stream for Network<D> {
  type Item = NetworkEvent<D>;

  fn poll_next(
    mut self: Pin<&mut Self>,
    cx: &mut Context<'_>,
  ) -> Poll<Option<Self::Item>> {
    if let Poll::Ready(Some(SwarmEvent::Behaviour(EpisubEvent::Message {
      topic,
      payload,
      ..
    }))) = self.swarm.poll_next_unpin(cx)
    {
      if topic == format!("/{}/vote", self.chainid) {
        match bincode::deserialize(&payload) {
          Ok(vote) => {
            return Poll::Ready(Some(NetworkEvent::VoteReceived(vote)))
          }
          Err(e) => error!("Failed to deserialize vote: {e}"),
        }
      } else if topic == format!("/{}/block", self.chainid) {
        match bincode::deserialize(&payload) {
          Ok(block) => {
            debug!("received block {block} through gossip");
            return Poll::Ready(Some(NetworkEvent::BlockReceived(block)));
          }
          Err(e) => error!("Failed to deserialize block: {e}"),
        }
      } else {
        warn!("something else on the network!");
      }
    }

    Poll::Pending
  }
}
