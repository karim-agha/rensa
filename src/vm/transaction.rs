use super::{contract::Environment, State};
use crate::primitives::{Keypair, Pubkey, ToBase58String};
use ed25519_dalek::{Signature, Signer};
use serde::{Deserialize, Serialize};

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
  pub accounts: Vec<AccountRef>,
  pub params: Vec<u8>,
  pub signatures: Vec<Signature>,
}

impl Transaction {
  pub fn new(
    contract: Pubkey,
    accounts: Vec<AccountRef>,
    params: Vec<u8>,
    signers: &[Keypair],
  ) -> Self {
    let mut buffer = Vec::new();
    let mut signatures = Vec::new();

    buffer.append(&mut contract.to_vec());

    for accref in &accounts {
      buffer.append(&mut accref.address.to_vec());
      buffer.push(match accref.writable {
        true => 1,
        false => 0,
      });
    }
    buffer.append(&mut params.clone());

    for signer in signers {
      signatures.push(signer.sign(&buffer));
    }

    Self {
      contract,
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
}

impl std::fmt::Debug for Transaction {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("Transaction")
      .field("contract", &self.contract)
      .field("accounts", &self.accounts)
      .field("params", &self.params.as_slice().to_b58())
      .field("signatures", &self.signatures)
      .finish()
  }
}
