use ed25519_dalek::{Signature, Signer};
use multihash::{Hasher, Sha3_256};
use serde::{Deserialize, Serialize};

use super::{contract::Environment, State};
use crate::primitives::{Keypair, Pubkey, ToBase58String};

/// This is a parameter on the transaction that indicates that
/// a contract is going to touch this account. Only accounts
/// speciefied in the accounts list in a transaction can be
/// accessed by smart contracts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountRef {
  pub address: Pubkey,
  pub writable: bool,
}

/// Represents a single invocation of the state machine.
/// This is the smallest unit of computation on the blockchain.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transaction {
  pub contract: Pubkey,
  pub payer: Pubkey,
  pub accounts: Vec<AccountRef>,
  pub params: Vec<u8>,
  pub signatures: Vec<Signature>,
}

impl Transaction {
  pub fn new(
    contract: Pubkey,
    payer: &Keypair,
    accounts: Vec<AccountRef>,
    params: Vec<u8>,
    signers: &[&Keypair],
  ) -> Self {
    let mut hasher = Sha3_256::default();
    hasher.update(&contract);
    hasher.update(&payer.public());

    for accref in &accounts {
      hasher.update(&accref.address);
      hasher.update(&[match accref.writable {
        true => 1,
        false => 0,
      }]);
    }
    hasher.update(&params);
    let fields_hash = hasher.finalize();

    // payers signature alwyas goes first
    let mut signatures = vec![payer.sign(fields_hash.as_ref())];

    // then signatures of all writable account owners
    for signer in signers {
      signatures.push(signer.sign(fields_hash.as_ref()));
    }

    Self {
      contract,
      payer: payer.public(),
      accounts,
      params,
      signatures,
    }
  }

  /// Creates a self-contained environment object that can be used to
  /// invoke a contract at a given version of the blockchain state.
  pub fn create_environment(&self, state: &impl State) -> Environment {
    Environment {
      address: self.contract.clone(),
      accounts: self
        .accounts
        .iter()
        .map(|a| (a.address.clone(), state.get(&a.address).cloned()))
        .collect(),
    }
  }

  pub fn hash(&self) -> Vec<u8> {
    let mut hasher = Sha3_256::default();
    hasher.update(&self.contract);
    hasher.update(&self.payer);
    for accref in &self.accounts {
      hasher.update(&accref.address);
      hasher.update(&[accref.writable as u8]);
    }
    hasher.update(&self.params);
    for sig in &self.signatures {
      hasher.update(sig.as_ref());
    }
    hasher.finalize().as_ref().to_vec()
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
