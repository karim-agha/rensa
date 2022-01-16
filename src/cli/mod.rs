use crate::keys::Keypair;
use clap::Parser;
use libp2p::{multiaddr::Protocol, Multiaddr};
use std::net::IpAddr;

#[derive(Debug, Parser)]
#[clap(version, about)]
pub struct CliOpts {
  #[clap(short, long, help = "secret key of the validator account")]
  pub secret: Keypair,

  #[clap(
    short,
    long,
    help = "listen address of the validator",
    default_value = "0.0.0.0"
  )]
  pub addr: IpAddr,

  #[clap(
    short,
    long,
    help = "listen port of the validator",
    default_value = "44668"
  )]
  pub port: u16,

  #[clap(
    short,
    long,
    parse(from_occurrences),
    help = "Use verbose output (-vv very verbose output)"
  )]
  pub verbose: u64,
}

impl CliOpts {
  pub fn listen_multiaddr(&self) -> Multiaddr {
    let mut maddr = Multiaddr::empty();
    maddr.push(match self.addr {
      IpAddr::V4(addr) => Protocol::Ip4(addr),
      IpAddr::V6(addr) => Protocol::Ip6(addr),
    });
    maddr.push(Protocol::Tcp(self.port));
    maddr
  }

  /// The libp2p identity of this validator node.
  /// This is based on the keypair provided through [`self.secret`]
  pub fn p2p_identity(&self) -> libp2p::identity::Keypair {
    libp2p::identity::Keypair::Ed25519(
      libp2p::identity::ed25519::SecretKey::from_bytes(
        &mut self.secret.secret.to_bytes(),
      )
      .unwrap()
      .into(),
    )
  }
}
