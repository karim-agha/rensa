pub trait ToBase58String {
  fn to_b58(&self) -> String;
}

impl<const S: usize> ToBase58String for multihash::MultihashGeneric<S> {
  fn to_b58(&self) -> String {
    bs58::encode(self.to_bytes()).into_string()
  }
}

impl ToBase58String for ed25519_dalek::Signature {
  fn to_b58(&self) -> String {
    bs58::encode(self.to_bytes()).into_string()
  }
}

impl ToBase58String for &[u8] {
  fn to_b58(&self) -> String {
    bs58::encode(self).into_string()
  }
}

impl ToBase58String for Vec<u8> {
  fn to_b58(&self) -> String {
    bs58::encode(self).into_string()
  }
}

// todo: turn those into macros
pub mod serde {
  use {
    crate::primitives::Pubkey,
    ::multihash::Multihash,
    ed25519_dalek::Signature,
    serde::{Deserialize, Deserializer, Serialize, Serializer},
  };

  pub fn serialize<S: Serializer>(
    v: &impl AsRef<[u8]>,
    s: S,
  ) -> Result<S::Ok, S::Error> {
    let b58 = bs58::encode(v).into_string();
    String::serialize(&b58, s)
  }

  pub fn deserialize<'de, D: Deserializer<'de>>(
    d: D,
  ) -> Result<Vec<u8>, D::Error> {
    let b58 = String::deserialize(d)?;
    bs58::decode(b58.as_bytes())
      .into_vec()
      .map_err(serde::de::Error::custom)
  }

  pub mod signature {
    use super::*;

    pub fn serialize<S: Serializer>(
      v: &Signature,
      s: S,
    ) -> Result<S::Ok, S::Error> {
      let b58 = bs58::encode(v).into_string();
      String::serialize(&b58, s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
      d: D,
    ) -> Result<Signature, D::Error> {
      let b58 = String::deserialize(d)?;
      Signature::from_bytes(
        &bs58::decode(b58.as_bytes())
          .into_vec()
          .map_err(serde::de::Error::custom)?,
      )
      .map_err(serde::de::Error::custom)
    }
  }

  pub mod signatures {
    use super::*;

    #[derive(Serialize, Deserialize)]
    struct Wrapper(#[serde(with = "super::signature")] Signature);

    pub fn serialize<S: Serializer>(
      v: &[Signature],
      s: S,
    ) -> Result<S::Ok, S::Error> {
      let sigs = v.iter().map(|s| Wrapper(*s)).collect();
      Vec::serialize(&sigs, s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
      d: D,
    ) -> Result<Vec<Signature>, D::Error> {
      Ok(
        Vec::deserialize(d)?
          .into_iter()
          .map(|Wrapper(a)| a)
          .collect(),
      )
    }
  }

  pub mod validator {
    use super::*;

    #[derive(Serialize, Deserialize)]
    struct Wrapper(Pubkey, #[serde(with = "super::signature")] Signature);

    pub fn serialize<S: Serializer>(
      (vp, vs): &(Pubkey, Signature),
      s: S,
    ) -> Result<S::Ok, S::Error> {
      let wrapped = Wrapper(*vp, *vs);
      Wrapper::serialize(&wrapped, s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
      d: D,
    ) -> Result<(Pubkey, Signature), D::Error> {
      let Wrapper(pubkey, sig) = Wrapper::deserialize(d)?;
      Ok((pubkey, sig))
    }
  }

  pub mod multihash {
    use super::*;

    pub fn serialize<S: Serializer>(
      v: &Multihash,
      s: S,
    ) -> Result<S::Ok, S::Error> {
      let b58 = bs58::encode(v.to_bytes()).into_string();
      String::serialize(&b58, s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
      d: D,
    ) -> Result<Multihash, D::Error> {
      let b58 = String::deserialize(d)?;
      Multihash::from_bytes(
        &bs58::decode(b58.as_bytes())
          .into_vec()
          .map_err(serde::de::Error::custom)?,
      )
      .map_err(serde::de::Error::custom)
    }
  }
}
