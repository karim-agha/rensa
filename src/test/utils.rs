use {
  crate::{
    consensus::{
      block::BlockData,
      genesis::Limits,
      validator::Validator,
      Genesis,
    },
    primitives::{Keypair, Pubkey},
  },
  chrono::Utc,
  ed25519_dalek::{PublicKey, SecretKey},
  std::{collections::BTreeMap, marker::PhantomData, time::Duration},
};

// TODO(bmaas): as these are used to configure the native callbacks, we should
// get these from somewhere deeper in the system
lazy_static::lazy_static! {
    pub static ref CURRENCY_CONTRACT_ADDR: Pubkey = "Currency1xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx".parse().unwrap();
}

pub fn genesis_default<D: BlockData>(keypair: &Keypair) -> Genesis<D> {
  Genesis::<D> {
    chain_id: "1".to_owned(),
    epoch_blocks: 32,
    genesis_time: Utc::now(),
    slot_interval: Duration::from_secs(2),
    state: BTreeMap::new(),
    builtins: vec![*CURRENCY_CONTRACT_ADDR],
    limits: Limits {
      max_block_size: 100_000,
      max_justification_age: 100,
      minimum_stake: 100,
      max_log_size: 512,
      max_logs_count: 32,
      max_account_size: 65536,
      max_input_accounts: 32,
      max_block_transactions: 2000,
      max_contract_size: 614400,
      max_transaction_params_size: 2048,
    },
    system_coin: "RensaToken1xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
      .parse()
      .unwrap(),
    validators: vec![Validator {
      pubkey: keypair.public(),
      stake: 200000,
    }],
    _marker: PhantomData,
  }
}

pub fn keypair_default() -> Keypair {
  let secret = SecretKey::from_bytes(&[
    157, 97, 177, 157, 239, 253, 90, 96, 186, 132, 74, 244, 146, 236, 44, 196,
    68, 73, 197, 105, 123, 50, 105, 25, 112, 59, 172, 3, 28, 174, 127, 96,
  ])
  .unwrap();
  let public: PublicKey = (&secret).into();
  let keypair: Keypair = ed25519_dalek::Keypair { secret, public }.into();
  keypair
}
