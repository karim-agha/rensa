use {
  super::{error::EpisubHandlerError, rpc},
  asynchronous_codec::{Bytes, BytesMut, Decoder, Encoder},
  prost::Message,
  unsigned_varint::codec,
};

/// All Episub messages are length-prefixed protobuf serialized bytes.
/// The length prefix is a varint. The protobuf schema of the protocol
/// is in rpc.proto.
pub struct EpisubCodec {
  /// Codec to encode/decode the Unsigned varint length prefix of the frames.
  length_codec: codec::UviBytes,
}

impl EpisubCodec {
  pub fn new(length_codec: codec::UviBytes) -> Self {
    Self { length_codec }
  }
}

impl Encoder for EpisubCodec {
  type Error = EpisubHandlerError;
  type Item = rpc::Rpc;

  fn encode(
    &mut self,
    item: Self::Item,
    dst: &mut BytesMut,
  ) -> Result<(), Self::Error> {
    // reserve output buffer
    let mut buf = Vec::with_capacity(item.encoded_len());

    // encode to protobuf
    item.encode(&mut buf).expect("buffer overrun");

    // and prefix it with length, fails if message is oversized
    self
      .length_codec
      .encode(Bytes::from(buf), dst)
      .map_err(|_| EpisubHandlerError::MaxTransmissionSize)
  }
}

impl Decoder for EpisubCodec {
  type Error = EpisubHandlerError;
  type Item = rpc::Rpc;

  fn decode(
    &mut self,
    src: &mut BytesMut,
  ) -> Result<Option<Self::Item>, Self::Error> {
    // prevent ddos by rejecting all oversized messages
    let packet = match self.length_codec.decode(src).map_err(|e| {
      if let std::io::ErrorKind::PermissionDenied = e.kind() {
        EpisubHandlerError::MaxTransmissionSize
      } else {
        EpisubHandlerError::Io(e)
      }
    })? {
      Some(p) => p,
      None => return Ok(None),
    };

    Ok(Some(
      rpc::Rpc::decode(&packet[..]).map_err(std::io::Error::from)?,
    ))
  }
}
