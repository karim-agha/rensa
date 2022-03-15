use {
  super::Pubkey,
  multihash::{
    Code as MultihashCode,
    Hasher,
    Multihash,
    MultihashDigest,
    Sha3_256,
  },
  serde::{Deserialize, Serialize},
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Account {
  #[serde(skip)]
  pub executable: bool,

  #[serde(default)]
  pub nonce: u64,
  
  pub owner: Option<Pubkey>,
  pub data: Option<Vec<u8>>,
}

impl Account {
  #[cfg(test)]
  pub fn test_new(value: u8) -> Self {
    Self {
      nonce: 1,
      executable: false,
      owner: None,
      data: Some(vec![value]),
    }
  }

  pub fn hash(&self) -> Multihash {
    let mut hasher = Sha3_256::default();
    hasher.update(&self.nonce.to_le_bytes());
    if let Some(ref owner) = self.owner {
      hasher.update(owner.as_ref());
    }
    if let Some(ref data) = self.data {
      hasher.update(data.as_ref());
    }
    MultihashCode::Sha3_256.wrap(hasher.finalize()).unwrap()
  }
}
