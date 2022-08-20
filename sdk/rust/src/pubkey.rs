use {
  borsh::{BorshDeserialize, BorshSerialize},
  std::{
    fmt::{Debug, Display},
    ops::Deref,
    str::FromStr,
  },
};

/// Represents an address of an account.
///
/// The same address could either represent a user wallet that
/// has a corresponding private key on the ed25519 curve or a
/// program owned account that is not on the curve and is writable
/// only by the contract owning it.
#[derive(
  Copy, Clone, PartialEq, Eq, PartialOrd, BorshSerialize, BorshDeserialize,
)]
pub struct Pubkey([u8; 32]);

impl AsRef<[u8]> for Pubkey {
  fn as_ref(&self) -> &[u8] {
    &self.0
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

impl TryFrom<&str> for Pubkey {
  type Error = bs58::decode::Error;

  fn try_from(value: &str) -> Result<Self, Self::Error> {
    FromStr::from_str(value)
  }
}
