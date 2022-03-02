use {
  crate::{consensus::Genesis, primitives::Keypair, vm::Transaction},
  clap::Parser,
  libp2p::{multiaddr::Protocol, Multiaddr},
  std::{
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    time::Duration,
  },
};

#[derive(Debug, Parser)]
#[clap(version, about)]
pub struct CliOpts {
  #[clap(short, long, help = "secret key of the validator account")]
  pub keypair: Keypair,

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
  peer: Vec<SocketAddr>,

  #[clap(
    long,
    help = "listen address of the validator",
    default_value = "0.0.0.0"
  )]
  addr: Vec<IpAddr>,

  #[clap(long, help = "listen port of the validator", default_value = "44668")]
  port: u16,

  #[clap(long, parse(from_os_str), help = "path to the chain genesis file")]
  genesis: PathBuf,

  #[clap(long, help = "port on which RPC API service is exposed")]
  rpc: Option<u16>,

  #[clap(
    long,
    parse(from_os_str),
    help = "path to the data directory",
    default_value = "~/.rensa/"
  )]
  data_dir: PathBuf,

  #[clap(
    long,
    help = "The number of N most recent block to store for block replays"
  )]
  replay_blocks_len: Option<u64>,
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

  /// Specifies how many blocks need to be persisted by a node to respond
  /// to block reply requests for other nodes. If the command line value
  /// is not provided, then the default is calculated to make sure that
  /// the node can replay any block within the last 30 minutes of confirmation.
  ///
  /// Peers that are at a point where they need replies older than that, will
  /// never catch up with the chain by replaying those blocks, instead they
  /// should trigger state sync to a more recent height and then replay the
  /// last few blocks.
  pub fn replay_blocks_len(&self) -> u64 {
    self.replay_blocks_len.unwrap_or_else(|| {
      let slot = self.genesis().unwrap().slot_interval.as_millis() as u64;
      let oldest = Duration::from_secs(30 * 60).as_millis() as u64; // 30 minutes
      oldest / slot
    })
  }
}
