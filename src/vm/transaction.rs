use {
  super::contract::ContractError,
  crate::{
    consensus::Limits,
    primitives::{Keypair, Pubkey, ToBase58String},
  },
  ed25519_dalek::{PublicKey, Signature, Signer, Verifier},
  multihash::{
    Code as MultihashCode,
    Hasher,
    Multihash,
    MultihashDigest,
    Sha3_256,
  },
  once_cell::sync::OnceCell,
  serde::{Deserialize, Serialize},
  std::io::{Error as StdError, ErrorKind},
  thiserror::Error,
};

/// This is a parameter on the transaction that indicates that
/// a contract is going to touch this account. Only accounts
/// speciefied in the accounts list in a transaction can be
/// accessed by smart contracts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountRef {
  pub address: Pubkey,
  pub writable: bool,
  pub signer: bool,
}

impl AccountRef {
  pub fn readonly(
    address: impl TryInto<Pubkey>,
    signer: bool,
  ) -> Result<AccountRef, StdError> {
    Ok(Self {
      address: address.try_into().map_err(|_| {
        StdError::new(ErrorKind::InvalidInput, "invalid pubkey")
      })?,
      writable: false,
      signer,
    })
  }

  pub fn writable(
    address: impl TryInto<Pubkey>,
    signer: bool,
  ) -> Result<AccountRef, StdError> {
    Ok(Self {
      address: address.try_into().map_err(|_| {
        StdError::new(ErrorKind::InvalidInput, "invalid pubkey")
      })?,
      writable: true,
      signer,
    })
  }
}

#[derive(Debug, Clone, Error, Serialize, Deserialize)]
pub enum TransactionError {
  #[error("The transaction is referencing too many accounts")]
  TooManyAccounts,

  #[error("The transaction's params blob is too big")]
  ParamsTooBig,
}

#[derive(Debug, Clone, Error, Serialize, Deserialize)]
pub enum SignatureError {
  #[error("Signature verification failed")]
  InvalidSignature,

  #[error("Missing Signers")]
  MissingSigners,
}

/// Represents a single invocation of the state machine.
/// This is the smallest unit of computation on the blockchain.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transaction {
  pub contract: Pubkey,
  pub nonce: u64,
  pub payer: Pubkey,
  pub accounts: Vec<AccountRef>,

  #[serde(with = "crate::primitives::b58::serde")]
  pub params: Vec<u8>,

  #[serde(with = "crate::primitives::b58::serde::signatures")]
  pub signatures: Vec<Signature>,

  #[serde(skip)]
  hashcache: OnceCell<Multihash>,
}

impl Transaction {
  pub fn new(
    contract: Pubkey,
    nonce: u64,
    payer: &Keypair,
    accounts: Vec<AccountRef>,
    params: Vec<u8>,
    signers: &[&Keypair],
  ) -> Self {
    let fields_hash =
      Self::hash_fields(&contract, nonce, &payer.public(), &accounts, &params);

    // payers signature alwyas goes first
    let mut signatures = vec![payer.sign(fields_hash.as_ref())];

    // then signatures of all writable account owners
    for signer in signers {
      signatures.push(signer.sign(fields_hash.as_ref()));
    }

    Self {
      contract,
      nonce,
      payer: payer.public(),
      accounts,
      params,
      signatures,
      hashcache: OnceCell::new(),
    }
  }

  pub fn verify_limits(&self, limits: &Limits) -> Result<(), TransactionError> {
    if self.accounts.len() > limits.max_input_accounts {
      return Err(TransactionError::TooManyAccounts);
    }

    if self.params.len() > limits.max_transaction_params_size {
      return Err(TransactionError::ParamsTooBig);
    }

    Ok(())
  }

  pub fn verify_signatures(&self) -> Result<(), SignatureError> {
    let fields_hash = Self::hash_fields(
      &self.contract,
      self.nonce,
      &self.payer,
      &self.accounts,
      &self.params,
    );

    // first verify the payer
    if self.signatures.is_empty() {
      // expecting at least one signature that
      // pays for transaction fees.
      return Err(SignatureError::MissingSigners);
    }

    if let Ok(payer_key) = PublicKey::from_bytes(&self.payer) {
      if payer_key
        .verify(fields_hash.as_ref(), self.signatures.first().unwrap())
        .is_err()
      {
        return Err(SignatureError::InvalidSignature);
      }
    } else {
      return Err(SignatureError::InvalidSignature);
    }

    // then the rest of signers
    let signing_accs = self
      .accounts
      .iter()
      .filter(|a| a.signer)
      .map(|a| &a.address);

    let expected_signatures = signing_accs.clone().count();
    let signers = signing_accs.zip(self.signatures.iter().skip(1));

    if signers.clone().count() != expected_signatures {
      return Err(SignatureError::MissingSigners);
    }

    for (pubkey, sig) in signers {
      if let Ok(pk) = PublicKey::from_bytes(pubkey) {
        if pk.verify(fields_hash.as_ref(), sig).is_ok() {
          continue;
        }
      }
      return Err(SignatureError::InvalidSignature);
    }

    Ok(())
  }

  pub fn hash(&self) -> &Multihash {
    self.hashcache.get_or_init(|| {
      let mut hasher = Sha3_256::default();
      hasher.update(&self.contract);
      hasher.update(&self.nonce.to_le_bytes());
      hasher.update(&self.payer);
      for accref in &self.accounts {
        hasher.update(&accref.address);
        hasher.update(&[accref.writable as u8]);
      }
      hasher.update(&self.params);
      for sig in &self.signatures {
        hasher.update(sig.as_ref());
      }
      MultihashCode::Sha3_256.wrap(hasher.finalize()).unwrap()
    })
  }

  fn hash_fields(
    contract: &Pubkey,
    nonce: u64,
    payer: &Pubkey,
    accounts: &[AccountRef],
    params: &[u8],
  ) -> Vec<u8> {
    let mut hasher = Sha3_256::default();
    hasher.update(contract);
    hasher.update(&nonce.to_le_bytes());
    hasher.update(payer);

    for accref in accounts {
      hasher.update(&accref.address);
      hasher.update(&[accref.writable as u8]);
      hasher.update(&[accref.signer as u8]);
    }
    hasher.update(params);
    hasher.finalize().to_vec()
  }
}

impl std::fmt::Debug for Transaction {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("Transaction")
      .field("contract", &self.contract)
      .field("payer", &self.payer)
      .field("accounts", &self.accounts)
      .field("params", &format!("{:02x?}", &self.params.as_slice()))
      .field(
        "signatures",
        &self
          .signatures
          .iter()
          .map(|s| format!("ed25519({})", s.to_b58()))
          .collect::<Vec<_>>(),
      )
      .field("hash", &self.hash().to_b58())
      .finish()
  }
}

impl std::fmt::Display for Transaction {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "{}", self.hash().to_b58())
  }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExecutedTransaction {
  pub block: u64,
  pub transaction: Transaction,
  pub output: Result<Vec<(String, String)>, ContractError>,
}
