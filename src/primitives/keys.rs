use {
  curve25519_dalek::edwards::CompressedEdwardsY,
  ed25519_dalek::{PublicKey, SecretKey},
  multihash::{Hasher, Sha3_256},
  serde::{
    de::{self, Visitor},
    Deserialize,
    Deserializer,
    Serialize,
  },
  std::{
    fmt::{Debug, Display, Formatter},
    marker::PhantomData,
    ops::Deref,
    str::FromStr,
  },
  thiserror::Error,
};

/// Represents an address of an account.
///
/// The same address could either represent a user wallet that
/// has a corresponding private key on the ed25519 curve or a
/// program owned account that is not on the curve and is writable
/// only by its program.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Pubkey([u8; 32]);

impl Pubkey {
  /// Given a list of seeds this method will generate a new
  /// derived pubkey that is not on the Rd25519 curve (and
  /// no private key exists).
  ///
  /// This method is used to generate addresses that are
  /// related to some original address.
  pub fn derive(&self, seeds: &[&[u8]]) -> Self {
    let mut bump: u32 = 0;
    loop {
      let mut hasher = Sha3_256::default();
      for seed in seeds.iter() {
        hasher.update(seed);
      }
      hasher.update(&bump.to_le_bytes());
      let key = Pubkey(hasher.finalize().try_into().unwrap());
      if !key.has_private_key() {
        return key;
      } else {
        bump += 1;
      }
    }
  }

  /// Checks if the given pubkey lies on the Ed25519 elliptic curve.
  ///
  /// When true, then it means that there exists a private key that
  /// make up together a valid Ed25519 keypair. Otherwise, when false
  /// it means that there is no corresponding valid private key.
  ///
  /// This is useful in cases we want to make sure that an account
  /// could not be ever modified except by its owning contract, as
  /// it is not possible to have a signer of a transaction that will
  /// give write access to an account.
  pub fn has_private_key(&self) -> bool {
    CompressedEdwardsY::from_slice(&self.0)
      .decompress()
      .is_some()
  }
}

impl Deref for Pubkey {
  type Target = [u8];

  fn deref(&self) -> &Self::Target {
    &self.0
  }
}

impl Display for Pubkey {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "{}", bs58::encode(self.0).into_string())
  }
}

impl Debug for Pubkey {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "Pubkey({})", bs58::encode(self.0).into_string())
  }
}

impl From<Pubkey> for String {
  fn from(pk: Pubkey) -> Self {
    bs58::encode(pk.0).into_string()
  }
}

impl FromStr for Pubkey {
  type Err = bs58::decode::Error;

  fn from_str(s: &str) -> Result<Self, Self::Err> {
    let mut bytes = [0u8; 32];
    bs58::decode(s).into(&mut bytes)?;
    Ok(Self(bytes))
  }
}

impl From<PublicKey> for Pubkey {
  fn from(p: PublicKey) -> Self {
    Self(*p.as_bytes())
  }
}

impl From<libp2p::PeerId> for Pubkey {
  fn from(p: libp2p::PeerId) -> Self {
    Self(p.as_ref().digest()[4..].try_into().unwrap())
  }
}

impl PartialEq<libp2p::PeerId> for Pubkey {
  fn eq(&self, other: &libp2p::PeerId) -> bool {
    self.0.eq(&other.as_ref().digest()[4..])
  }
}

impl PartialEq<Pubkey> for libp2p::PeerId {
  fn eq(&self, other: &Pubkey) -> bool {
    other.0.eq(&self.as_ref().digest()[4..])
  }
}

/// Represents a wallet account on the ed25519 curve that can
/// be controlled by an external wallet and cannot be owned by
/// a program.
pub struct Keypair(ed25519_dalek::Keypair);

impl Keypair {
  pub fn public(&self) -> Pubkey {
    self.0.public.into()
  }

  pub fn secret(&self) -> &SecretKey {
    &self.0.secret
  }
}

impl Clone for Keypair {
  fn clone(&self) -> Self {
    Self(ed25519_dalek::Keypair::from_bytes(&self.0.to_bytes()).unwrap())
  }
}

impl Deref for Keypair {
  type Target = ed25519_dalek::Keypair;

  fn deref(&self) -> &Self::Target {
    &self.0
  }
}

impl std::fmt::Debug for Keypair {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_tuple("Keypair").field(&self.0.public).finish()
  }
}

impl Display for Keypair {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(
      f,
      "Keypair({})",
      bs58::encode(self.0.public.as_bytes()).into_string()
    )
  }
}

impl From<ed25519_dalek::Keypair> for Keypair {
  fn from(k: ed25519_dalek::Keypair) -> Self {
    Self(k)
  }
}

impl From<Keypair> for ed25519_dalek::Keypair {
  fn from(kp: Keypair) -> Self {
    ed25519_dalek::Keypair::from_bytes(&kp.0.to_bytes()).unwrap()
  }
}

#[derive(Debug, Error)]
pub enum KeypairError {
  #[error("Failed parsing base58 string: {0:?}")]
  Base58ParseError(bs58::decode::Error),

  #[error("{0}")]
  Ed25519Error(#[from] ed25519_dalek::ed25519::Error),
}

impl TryFrom<&[u8]> for Keypair {
  type Error = KeypairError;

  fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
    let secret = SecretKey::from_bytes(value)?;
    let public: PublicKey = (&secret).into();
    Ok(Self(ed25519_dalek::Keypair { secret, public }))
  }
}

impl FromStr for Keypair {
  type Err = KeypairError;

  fn from_str(value: &str) -> Result<Self, Self::Err> {
    let mut secret = [0u8; 32];
    bs58::decode(value)
      .into(&mut secret)
      .map_err(KeypairError::Base58ParseError)?;
    let secret = SecretKey::from_bytes(&secret)?;
    let public = (&secret).into();
    Ok(Keypair(ed25519_dalek::Keypair { secret, public }))
  }
}

/// Deserialize a pubkey for either a user-friendsly base58
/// representation or a machine-friendly byte array.
impl<'de> Deserialize<'de> for Pubkey {
  fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
  where
    D: Deserializer<'de>,
  {
    struct StringOrArray<T>(PhantomData<fn() -> T>);

    impl<'de, T> Visitor<'de> for StringOrArray<T>
    where
      T: Deserialize<'de> + FromStr<Err = bs58::decode::Error>,
    {
      type Value = T;

      fn expecting(&self, formatter: &mut Formatter) -> std::fmt::Result {
        formatter.write_str("base58 string or byte array")
      }

      fn visit_str<E>(self, value: &str) -> Result<T, E>
      where
        E: de::Error,
      {
        FromStr::from_str(value)
          .map_err(|e| de::Error::custom(format!("{e:?}")))
      }

      fn visit_seq<A>(self, seq: A) -> Result<Self::Value, A::Error>
      where
        A: de::SeqAccess<'de>,
      {
        Deserialize::deserialize(de::value::SeqAccessDeserializer::new(seq))
      }
    }

    deserializer.deserialize_str(StringOrArray(PhantomData))
  }
}

impl Serialize for Pubkey {
  fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
  where
    S: serde::Serializer,
  {
    serializer.serialize_str(&bs58::encode(self.0).into_string())
  }
}

#[cfg(test)]
mod test {
  use super::Pubkey;

  #[test]
  fn pubkey_derive_some() {
    // corresponding private key: 9Rt2PJombdzAEjdgiybg4woayTwKVD89uYYc1vFy7Hoa
    let pk1: Pubkey = "GBQEQGo5zQYCFdewiWuZ5FT9pi6D4muTAvyYzqR4ty4U"
      .parse()
      .unwrap();
    assert!(pk1.has_private_key());

    let der1 = pk1.derive(&[b"some random seed"]);
    assert!(!der1.has_private_key());

    for i in 0..1000u32 {
      let pk = pk1.derive(&[&i.to_le_bytes()]);
      assert!(!pk.has_private_key());
    }
  }
}
