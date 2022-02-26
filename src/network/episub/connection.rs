use {
  super::{codec::EpisubCodec, error::EpisubHandlerError},
  asynchronous_codec::Framed,
  futures::{future, AsyncRead, AsyncWrite},
  libp2p::core::{InboundUpgrade, OutboundUpgrade, UpgradeInfo},
  std::{future::Future, iter, pin::Pin},
  unsigned_varint::codec,
};

#[derive(Debug, Clone)]
pub struct EpisubConnection {
  max_transmit_size: usize,
}

impl EpisubConnection {
  pub fn new(max_transmit_size: usize) -> Self {
    Self { max_transmit_size }
  }
}

impl UpgradeInfo for EpisubConnection {
  type Info = &'static [u8];
  type InfoIter = iter::Once<Self::Info>;

  fn protocol_info(&self) -> Self::InfoIter {
    iter::once(b"/rensa/episub/1.0.0")
  }
}

impl<TSocket> InboundUpgrade<TSocket> for EpisubConnection
where
  TSocket: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
  type Error = EpisubHandlerError;
  #[allow(clippy::type_complexity)] // oh well
  type Future =
    Pin<Box<dyn Future<Output = Result<Self::Output, Self::Error>> + Send>>;
  type Output = Framed<TSocket, EpisubCodec>;

  fn upgrade_inbound(self, socket: TSocket, _: Self::Info) -> Self::Future {
    let mut length_codec = codec::UviBytes::default();
    length_codec.set_max_len(self.max_transmit_size);
    Box::pin(future::ok(Framed::new(
      socket,
      EpisubCodec::new(length_codec),
    )))
  }
}

impl<TSocket> OutboundUpgrade<TSocket> for EpisubConnection
where
  TSocket: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
  type Error = EpisubHandlerError;
  #[allow(clippy::type_complexity)] // oh well
  type Future =
    Pin<Box<dyn Future<Output = Result<Self::Output, Self::Error>> + Send>>;
  type Output = Framed<TSocket, EpisubCodec>;

  fn upgrade_outbound(self, socket: TSocket, _: Self::Info) -> Self::Future {
    let mut length_codec = codec::UviBytes::default();
    length_codec.set_max_len(self.max_transmit_size);
    Box::pin(future::ok(Framed::new(
      socket,
      EpisubCodec::new(length_codec),
    )))
  }
}
