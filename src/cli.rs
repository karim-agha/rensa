use {
  crate::{consensus::Genesis, primitives::Keypair, vm::Transaction},
  clap::Parser,
  libp2p::{multiaddr::Protocol, Multiaddr},
  std::{
    net::{IpAddr, SocketAddr},
    path::PathBuf,
  },
};

#[derive(Debug, Parser)]
#[clap(version, about)]
pub struct CliOpts {
  #[clap(short, long, help = "secret key of the validator account")]
  pub keypair: Keypair,

  #[clap(
    long,
    help = "listen address of the validator",
    default_value = "0.0.0.0"
  )]
  pub addr: Vec<IpAddr>,

  #[clap(long, help = "listen port of the validator", default_value = "44668")]
  pub port: u16,

  #[clap(
    short,
    long,
    parse(from_occurrences),
    help = "Use verbose output (-vv very verbose output)"
  )]
  pub verbose: u64,

  #[clap(
    long,
    help = "address of a known peer to bootstrap p2p networking from"
  )]
  pub peer: Vec<SocketAddr>,

  #[clap(long, parse(from_os_str), help = "path to the chain genesis file")]
  pub genesis: PathBuf,

  #[clap(long, help = "port on which RPC API service is exposed")]
  pub rpc: Option<u16>,

  #[clap(
    long,
    parse(from_os_str),
    help = "path to the data directory",
    default_value = "~/.rensa/"
  )]
  pub data_dir: PathBuf,
}

impl CliOpts {
  /// Lists all the multiaddresses this node will listen
  /// on for incoming connections. By default it will listen
  /// on all available interfaces.
  pub fn listen_multiaddrs(&self) -> Vec<Multiaddr> {
    self
      .addr
      .iter()
      .map(|addr| {
        let mut maddr = Multiaddr::empty();
        maddr.push(match *addr {
          IpAddr::V4(addr) => Protocol::Ip4(addr),
          IpAddr::V6(addr) => Protocol::Ip6(addr),
        });
        maddr.push(Protocol::Tcp(self.port));
        maddr
      })
      .collect()
  }

  /// Lists all multiaddresses of known peers of the chain.
  /// Those peers are used as first bootstrap nodes to join
  /// the p2p gossip network of the chain.
  pub fn peers(&self) -> Vec<Multiaddr> {
    self
      .peer
      .iter()
      .map(|addr| {
        let mut maddr = Multiaddr::empty();
        maddr.push(match *addr {
          SocketAddr::V4(addr) => Protocol::Ip4(*addr.ip()),
          SocketAddr::V6(addr) => Protocol::Ip6(*addr.ip()),
        });
        maddr.push(Protocol::Tcp(addr.port()));
        maddr
      })
      .collect()
  }

  /// The libp2p identity of this validator node.
  /// This is based on the keypair provided through [`self.secret`]
  pub fn p2p_identity(&self) -> libp2p::identity::Keypair {
    libp2p::identity::Keypair::Ed25519(
      libp2p::identity::ed25519::SecretKey::from_bytes(
        &mut self.keypair.secret().to_bytes(),
      )
      .unwrap()
      .into(),
    )
  }

  /// Retreives the genesis block config from its JSON
  /// serialized form from the path provided by the user.
  pub fn genesis(&self) -> Result<Genesis<Vec<Transaction>>, std::io::Error> {
    let json =
      std::fs::read_to_string(&self.genesis).map_err(std::io::Error::from)?;
    let mut genesis: Genesis<Vec<Transaction>> =
      serde_json::from_str(&json).map_err(std::io::Error::from)?;

    // we're sorting validators in the genesis because we want the same
    // hash for two gensis files with the exact same list of parameters
    // and validators but only differing in the order of their appearance.
    genesis.validators.sort();
    Ok(genesis)
  }

  /// If an RPC port is provided provided, returns all socketaddrs on which
  /// the RPC API service will be listening on incoming JSON-RPC calls from
  /// external clients.
  pub fn rpc_endpoints(&self) -> Option<Vec<SocketAddr>> {
    self.rpc.map(|port| {
      self
        .addr
        .iter()
        .cloned()
        .map(|addr| SocketAddr::new(addr, port))
        .collect()
    })
  }

  /// Gets the data directory for the this chain.
  /// The chain directory is <top-level-data-dir>/<chain-id>/*
  pub fn data_dir(&self) -> Result<PathBuf, std::io::Error> {
    let chain_id = self.genesis()?.chain_id;
    let mut dir: PathBuf = shellexpand::full(self.data_dir.to_str().unwrap())
      .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?
      .to_string()
      .into();
    dir.push(chain_id);
    std::fs::create_dir_all(dir.clone())?;
    Ok(dir)
  }
}
