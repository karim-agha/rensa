use {
  crate::{
    primitives::{Keypair, Pubkey},
    test::utils::CURRENCY_CONTRACT_ADDR,
    vm::{AccountRef, Transaction},
  },
  borsh::BorshSerialize,
};

/// Abstract class to generate a currency
pub struct Currency {
  // pub mint: Pubkey,
}

// TODO: client/js/src/currency.ts implement currency transfers
// TODO: can we have tokens without any symbol?

impl Currency {
  pub fn create(
    payer: Keypair,
    nonce: u64,
    seed: &[u8; 32],
    authority: Pubkey,
    decimals: u8,
    name: Option<String>,
    symbol: Option<String>,
  ) -> Transaction {
    let ix = crate::vm::builtin::currency::Instruction::Create {
      seed: seed.clone(),
      authority,
      decimals,
      name,
      symbol,
    };

    let params = ix.try_to_vec().unwrap();

    let mint_address = CURRENCY_CONTRACT_ADDR.derive(&[seed]);

    let accounts = vec![AccountRef {
      address: mint_address,
      signer: false,
      writable: true,
    }];

    return Transaction::new(
      *CURRENCY_CONTRACT_ADDR,
      nonce,
      &payer,
      accounts,
      params,
      &[&payer],
    );
  }

  // fn mint(&self, authority: Keypair, payer: Keypair, amount: u64) {}
}

/// Helper to quickly create the PQ Token
pub fn create_pq_token_tx(payer: &Keypair) -> Transaction {
  Currency::create(
    payer.clone(),
    1,
    &[0; 32],
    payer.public(),
    9,
    Some(String::from("PQ Token")),
    Some(String::from("PQ")),
  )
}
