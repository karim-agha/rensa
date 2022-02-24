mod episub;

use {
  crate::{
    consensus::{Block, BlockData, Genesis, Produced, Vote},
    primitives::{Keypair, Pubkey},
  },
  episub::{Config, Episub, EpisubEvent, PeerAuthorizer},
  futures::StreamExt,
  libp2p::{
    core::{muxing::StreamMuxerBox, transport::Boxed, upgrade::Version},
    dns::{DnsConfig, ResolverConfig, ResolverOpts},
    identity::{self, ed25519::SecretKey},
    noise,
    swarm::SwarmEvent,
    tcp::TcpConfig,
    yamux::YamuxConfig,
    Multiaddr,
    PeerId,
    Swarm,
    Transport,
  },
  std::collections::HashSet,
  tokio::sync::mpsc::{
    error::SendError,
    unbounded_channel,
    UnboundedReceiver,
    UnboundedSender,
  },
  tracing::{debug, error, warn},
};

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
  BlockReceived(Produced<D>),
  VoteReceived(Vote),
}
// this is a bug in clippy, I filed an issue on GH:
// https://github.com/rust-lang/rust-clippy/issues/8321
// remove this when the issue gets closed.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum NetworkCommand<D: BlockData> {
  Connect(Multiaddr),
  GossipBlock(Produced<D>),
  GossipVote(Vote),
}

pub struct Network<D: BlockData> {
  netin: UnboundedReceiver<NetworkEvent<D>>,
  netout: UnboundedSender<NetworkCommand<D>>,
}

impl<D: BlockData> Network<D> {
  pub async fn new(
    genesis: &Genesis<D>,
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
      .filter(|v| v.stake >= genesis.minimum_stake)
      .map(|v| v.pubkey)
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
        active_view_factor: 4,
        max_transmit_size: genesis.max_block_size,
        // 2 epochs are needed until block finalization
        history_window: genesis.slot_interval
          * (genesis.epoch_slots as u32 * 2),
        // keep informing all peers about all messages received for the last
        // epoch
        network_size: genesis.validators.len(),
        lazy_push_interval: genesis.slot_interval * genesis.epoch_slots as u32,
        ..Config::default()
      }),
      id.public().to_peer_id(),
    );

    let chainid = genesis.chain_id.clone();

    swarm
      .behaviour_mut()
      .subscribe(format!("/{}/vote", &chainid));
    swarm
      .behaviour_mut()
      .subscribe(format!("/{}/block", &chainid));
    swarm.behaviour_mut().subscribe(format!("/{}/tx", &chainid));

    listenaddrs.for_each(|addr| {
      swarm.listen_on(addr).unwrap();
    });

    let (netin_tx, netin_rx) = unbounded_channel();
    let (netout_tx, mut netout_rx) = unbounded_channel();

    tokio::spawn(async move {
      loop {
        tokio::select! {
          Some(event) = swarm.next() => {
            if let SwarmEvent::Behaviour(EpisubEvent::Message {
              topic,
              payload,
              ..
            }) = event
            {
              if topic == format!("/{}/vote", chainid) {
                match bincode::deserialize(&payload) {
                  Ok(vote) => {
                    netin_tx.send(NetworkEvent::VoteReceived(vote)).unwrap();
                  }
                  Err(e) => error!("Failed to deserialize vote: {e}"),
                }
              } else if topic == format!("/{}/block", chainid) {
                match bincode::deserialize(&payload) {
                  Ok(block) => {
                    debug!("received block {block} through gossip");
                    netin_tx.send(NetworkEvent::BlockReceived(block)).unwrap();
                  }
                  Err(e) => error!("Failed to deserialize block: {e}"),
                }
              } else {
                warn!("Received a message on an unexpected topic {topic}");
              }
            }
          },
          Some(event) = netout_rx.recv() => {
            match event {
              NetworkCommand::Connect(addr)=>{
                if let Err(e) = swarm.dial(addr.clone()) {
                  error!("Dialing peer {addr} failed: {e}");
                }
              }
              NetworkCommand::GossipBlock(block) => {
                swarm
                .behaviour_mut()
                .publish(
                  &format!("/{}/block", chainid),
                  block.to_bytes().expect("Produced malformed block"))
                .unwrap();
              },
              NetworkCommand::GossipVote(vote) => {
                swarm
                .behaviour_mut()
                .publish(
                  &format!("/{}/vote", chainid),
                  vote.to_bytes().expect("Produced malformed vote"))
                .unwrap();
              }
            }
          }
        }
      }
    });

    Ok(Self {
      netin: netin_rx,
      netout: netout_tx,
    })
  }

  pub fn connect(
    &mut self,
    addr: Multiaddr,
  ) -> Result<(), SendError<NetworkCommand<D>>> {
    self.netout.send(NetworkCommand::Connect(addr))
  }

  pub fn gossip_vote(
    &mut self,
    vote: Vote,
  ) -> Result<(), SendError<NetworkCommand<D>>> {
    self.netout.send(NetworkCommand::GossipVote(vote))
  }

  pub fn gossip_block(
    &mut self,
    block: Produced<D>,
  ) -> Result<(), SendError<NetworkCommand<D>>> {
    self.netout.send(NetworkCommand::GossipBlock(block))
  }

  pub async fn poll(&mut self) -> Option<NetworkEvent<D>> {
    self.netin.recv().await
  }
}
