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
      Executable,
      Executed,
      Finalized,
      State,
      StateDiff,
      Transaction,
    },
  },
  borsh::BorshSerialize,
  chrono::Utc,
  ed25519_dalek::{PublicKey, SecretKey},
  futures::StreamExt,
  multihash::Multihash,
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

  pub fn process_transaction(&mut self, tx: D) -> Produced<D> {
    let (parent, statehash) = self.chain.with_head(|s, b| {
      (
        b.hash().unwrap().clone(),
        tx.execute(&self.ctx.vm, s).unwrap().hash().clone(),
      )
    });

    let produced = Produced::new(
      &self.ctx.keypair,
      self.inc_height(),
      parent,
      tx,
      statehash,
      vec![],
    )
    .unwrap();

    // NOTE: we vote on a block hash thats why
    // we need an extra block
    let _produced_vote_block = Produced::new(
      &self.ctx.keypair,
      self.inc_height(),
      produced.hash().unwrap(),
      // StateDiff::default(),
      D::default(),
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

    // NOTE: since we are always adding from one validator
    // we do not have. The impact is we will never finalized
    // blocks will be in the blocktree. The blocks will in this case be in the
    // head of the forktree.

    // self.chain.include(produced_vote_block);

    // NOTE: one solution will be to listen to events
    // in this case ChainEvent(BlockIncluded)
    // let next_event = self.chain.next().await;
    //
    // TODO: downcast function on Block
    // https://bennetthardwick.com/rust/downcast-trait-object/

    // downcast block headblock in the forktree to a cloned concrete
    // type.
    // TODO: ask karim if we could also just return the produced
    // block from before?
    let block = self
      .chain
      .with_head(|_, b| b.as_any().downcast_ref::<Produced<D>>().cloned())
      .expect("headblock is not a produced, add some more options here?");

    // now we are on an interesting part of the system. The head block
    // is stored in memory, and the finalized is actually finalized
    // in the persistent store. For testing we want access to the state
    // and we want to get for erample the value of an account.

    // dependable on the fact we want finalized or not finalized we
    // can have a unique accesssor

    block
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
      0,
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

    validator.process_transaction(vec![]);
  }

  #[test]
  fn process_transaction_test() {
    let ctx: TestCtx<Vec<Transaction>> = TestCtx::new();
    let mut validator = TestValidator::new(&ctx);

    // NOTE: a way to generate a random keypair
    let payer = keypair_default();

    // TODO: client/js/src/currency.ts implement currency transfers
    // TODO: can we have tokens without any symbol?
    let tx_create =
      Currency::create(payer.clone(), &[0; 32], payer.public(), 9, None, None);

    let block = validator.process_transaction(vec![tx_create]);
    dbg!(&block);
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
