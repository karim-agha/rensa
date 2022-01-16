use libp2p::{
  core::{muxing::StreamMuxerBox, transport::Boxed, upgrade::Version},
  dns::{DnsConfig, ResolverConfig, ResolverOpts},
  identity::{self, ed25519::SecretKey},
  noise,
  tcp::TcpConfig,
  yamux::YamuxConfig,
  PeerId, Transport,
};

use crate::keys::Keypair;

pub type BoxedTransport = Boxed<(PeerId, StreamMuxerBox)>;

pub async fn create_transport(
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
