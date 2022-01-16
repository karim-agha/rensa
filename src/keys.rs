use base58::{FromBase58, FromBase58Error, ToBase58};
use ed25519_dalek::{PublicKey, SecretKey};
use std::{fmt::Display, ops::Deref, str::FromStr};
use thiserror::Error;

/// Represents an address of an account
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Pubkey([u8; 32]);

impl Deref for Pubkey {
  type Target = [u8];
  fn deref(&self) -> &Self::Target {
    &self.0
  }
}

impl Display for Pubkey {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "{}", self.0.to_base58())
  }
}

impl Into<String> for Pubkey {
  fn into(self) -> String {
    self.0.to_base58()
  }
}

impl TryFrom<String> for Pubkey {
  type Error = FromBase58Error;
  fn try_from(value: String) -> Result<Self, Self::Error> {
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&value.from_base58()?[..]);
    Ok(Self(bytes))
  }
}

impl From<PublicKey> for Pubkey {
  fn from(p: PublicKey) -> Self {
    Self(*p.as_bytes())
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
    write!(f, "Keypair({})", self.0.public.as_bytes().to_base58())
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
  Base58ParseError(FromBase58Error),

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
    let bytes = &value
      .from_base58()
      .map_err(KeypairError::Base58ParseError)?[..];
    Ok(bytes.try_into()?)
  }
}
