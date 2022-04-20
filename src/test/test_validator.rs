use {
  crate::{
    consensus::{
      block::{Block, BlockData, Produced},
      forktree::{TreeNode, VolatileBlock},
      genesis::Limits,
      validator::Validator,
      Chain,
      Genesis,
      Vote,
    },
    primitives::{Account, Keypair, Pubkey},
    storage::PersistentState,
    vm::{
      self,
      AccountRef,
      BlockOutput,
      ContractError,
      Executable,
      Executed,
      Finalized,
      MachineError,
      State,
      StateDiff,
      Transaction,
    },
  },
  borsh::BorshSerialize,
  chrono::Utc,
  ed25519_dalek::{PublicKey, SecretKey},
  futures::StreamExt,
  indexmap::{map::Values, IndexMap},
  multihash::{Multihash, MultihashGeneric},
  rand::{distributions::Alphanumeric, thread_rng, Rng},
  std::{
    any::Any,
    collections::BTreeMap,
    marker::PhantomData,
    sync::Arc,
    time::Duration,
  },
};

/// Implements TestCtx
pub struct TestCtx<D: BlockData> {
  genesis: Genesis<D>,
  store: PersistentState,
  vm: vm::Machine,
  keypair: Keypair,
}

impl<D: BlockData> TestCtx<D> {
  pub fn new() -> Self {
    let keypair = keypair_default();
    let genesis = genesis_default::<D>(&keypair);

    // build persistent state, we generate a random dir
    // for each instance of TestCtx
    let mut randomdir = std::env::temp_dir();
    randomdir.push(
      &thread_rng()
        .sample_iter(&Alphanumeric)
        .take(16)
        .map(char::from)
        .collect::<String>(),
    );
    let store = PersistentState::new(&genesis, randomdir.clone()).unwrap();

    let vm = vm::Machine::new(&genesis).unwrap();

    Self {
      genesis,
      store,
      vm,
      keypair,
    }
  }
}

pub struct ProcessTransactionsResult<D: BlockData> {
  block: Produced<D>,
  block_output: BlockOutput,
  transactions: D,
}

impl<D: BlockData> ProcessTransactionsResult<D> {
  /// returns the resulting logs after processing the transaction
  fn logs(&self) -> &IndexMap<Multihash, Vec<(String, String)>> {
    &*self.block_output.logs
  }

  /// returns the resulting errors after executing transactions
  fn errors(&self) -> &IndexMap<Multihash, ContractError> {
    &*self.block_output.errors
  }

  /// returns the resulting StateDiff after processing the transactions
  fn state(&self) -> &StateDiff {
    &self.block_output.state
  }
}

/// Implements a TestValidator
/// it will generate a executed block on each transaction and will vote for it
pub struct TestValidator<'g, D: BlockData> {
  ctx: &'g TestCtx<D>,
  chain: Chain<'g, D>,
  height: u64,
}

impl<'g, D: BlockData> TestValidator<'g, D> {
  pub fn new(ctx: &'g TestCtx<D>) -> Self {
    let finalized = Finalized::new(Arc::new(ctx.genesis.clone()), &ctx.store);
    let chain = Chain::new(&ctx.genesis, &ctx.vm, finalized);

    Self {
      ctx: &ctx,
      chain,
      height: 0,
    }
  }

  fn inc_height(&mut self) -> u64 {
    self.height += 1;
    self.height
  }

  pub fn get_account(&self, pubkey: &Pubkey) -> Option<Account> {
    self.chain.with_head(|s, _| s.get(pubkey))
  }

  pub fn add_account(&self, pubkey: Pubkey, account: Account) {
    let mut diff = StateDiff::default();
    diff.set(pubkey, account).unwrap();
    self.ctx.store.apply(diff).unwrap();
  }

  pub fn delete_account(&self, pubkey: Pubkey) {
    let mut diff = StateDiff::default();
    diff.remove(pubkey).unwrap();
    self.ctx.store.apply(diff).unwrap();
  }

  pub fn process_transactions(
    &mut self,
    transactions: D,
  ) -> Result<ProcessTransactionsResult<D>, MachineError> {
    // execute our transaction on the head state and return the
    // parents hash
    let (parent, execution_result) = self.chain.with_head(|s, b| {
      (
        b.hash().unwrap().clone(),
        transactions.execute(&self.ctx.vm, s),
      )
    });

    let block_output = execution_result?;
    let statehash = block_output.hash().clone();

    // produce a new new block
    let produced = Produced::new(
      &self.ctx.keypair,
      self.inc_height(),
      parent,
      transactions.clone(),
      statehash,
      vec![],
    )
    .unwrap();

    // produce a new vote block, to be able to finalize
    // the previous block
    let _produced_vote_block = Produced::new(
      &self.ctx.keypair,
      self.inc_height(),
      produced.hash().unwrap(),
      D::default(), // StateDiff::default(),
      statehash,
      vec![Vote::new(
        &self.ctx.keypair,
        produced.hash().unwrap(),
        self.ctx.genesis.hash().unwrap(),
      )],
    )
    .unwrap();

    // include the block, and ensure our single validator
    // votes on the block
    self.chain.include(produced);

    // we are now not including the produced vote block
    // which will mean we will never finalize blocks. And
    // thus also never storing the state into peristen state.

    // self.chain.include(produced_vote_block); <== enable this to finalize

    // TODO: ask karim if we could also just return the produced
    // block from before?
    let block = self
      .chain
      .with_head(|_, b| b.as_any().downcast_ref::<Produced<D>>().cloned())
      .expect("headblock is not a produced, add some more options here?");

    // Return our result
    Ok(ProcessTransactionsResult {
      block_output,
      block,
      transactions,
    })
  }
}

lazy_static::lazy_static! {
    static ref CURRENCY_CONTRACT_ADDR: Pubkey = "Currency1xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx".parse().unwrap();
}

pub fn genesis_default<D: BlockData>(keypair: &Keypair) -> Genesis<D> {
  let genesis = Genesis::<D> {
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
  };
  genesis
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

/// Abstract class to generate a currency
struct Currency {
  mint: Pubkey,
}

impl Currency {
  fn create(
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

  fn mint(&self, authority: Keypair, payer: Keypair, amount: u64) {}
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn initialize_test_validator_test() {
    let ctx: TestCtx<Vec<Transaction>> = TestCtx::new();
    let mut validator = TestValidator::new(&ctx);

    validator.process_transactions(vec![]).unwrap();
  }

  #[test]
  fn process_transactions_test() {
    let ctx: TestCtx<Vec<Transaction>> = TestCtx::new();
    let mut validator = TestValidator::new(&ctx);

    // NOTE: a way to generate a random keypair
    let payer = keypair_default();

    // TODO: client/js/src/currency.ts implement currency transfers
    // TODO: can we have tokens without any symbol?
    let tx_create = Currency::create(
      payer.clone(),
      1,
      &[0; 32],
      payer.public(),
      9,
      Some(String::from("PQ Token")),
      Some(String::from("PQ")),
    );

    let result = validator.process_transactions(vec![tx_create]).unwrap();

    dbg!(&result.logs().values());
    dbg!(&result.errors().values());
    dbg!(&result.state());
    // dbg!(&block_output.state);
    // dbg!(&*block_output.logs);
    // dbg!(&*block_output.errors);
  }

  // TODO: this has to be tested in a unit test
  // assert!(statehash.is_valid_ipfs_cid());
  // assert!(statehash.decode_cid().parent == parent);
  // assert that inline data < 256KB
  // assert if data > 256KB, CID links sum of links == size of Data
  // Cid
  // Multiformat
  // Multibase
  // PB-DAG (Protobuf-DAG)0
}
