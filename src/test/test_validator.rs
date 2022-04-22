use {
  crate::{
    consensus::{
      block::{Block, BlockData, Produced},
      Chain,
      Genesis,
      Vote,
    },
    primitives::{b58::ToBase58String, Account, Keypair, Pubkey},
    storage::{Error as StorageError, PersistentState},
    test::{
      currency::create_pq_token_tx,
      utils::{genesis_default, keypair_default},
    },
    vm::{
      self,
      ApplicableState,
      BlockOutput,
      ContractError,
      Finalized,
      MachineError,
      State,
      StateDiff,
      StateError,
      Transaction,
    },
  },
  indexmap::IndexMap,
  multihash::Multihash,
  rand::{distributions::Alphanumeric, thread_rng, Rng},
  std::{cell::RefCell, collections::HashMap, sync::Arc},
};

#[derive(Debug, Default)]
pub struct InMemState {
  db: RefCell<HashMap<Pubkey, Account>>,
}

impl State for InMemState {
  fn get(&self, address: &Pubkey) -> Option<Account> {
    todo!()
  }

  fn set(
    &mut self,
    address: Pubkey,
    account: Account,
  ) -> Result<Option<Account>, StateError> {
    todo!()
  }

  fn remove(&mut self, address: Pubkey) -> Result<(), StateError> {
    todo!()
  }

  fn hash(&self) -> Multihash {
    todo!()
  }
}

impl ApplicableState for InMemState {
  fn apply(&self, diff: StateDiff) -> std::result::Result<(), StorageError> {
    let mut db = self.db.borrow_mut();
    for (addr, account) in diff.into_iter() {
      match account {
        Some(account) => db.insert(addr, account),
        None => db.remove(&addr),
      };
    }

    Ok(())
  }
}

/// Construct a TestCtx based on a Genesis
pub struct TestCtx<D: BlockData> {
  genesis: Genesis<D>,
  store: InMemState,
  vm: vm::Machine,
  keypair: Keypair,
}

impl<D: BlockData> TestCtx<D> {
  pub fn new() -> Self {
    let keypair = keypair_default();
    let genesis = genesis_default::<D>(&keypair);

    // build persistent state, we generate a random dir
    // for each instance of TestCtx
    //
    // TODO(bmaas): create a test double for PersistState, an in
    // memory storage. This will remove the need for this randomdir
    let mut randomdir = std::env::temp_dir();
    randomdir.push(
      &thread_rng()
        .sample_iter(&Alphanumeric)
        .take(16)
        .map(char::from)
        .collect::<String>(),
    );
    // let store = PersistentState::new(&genesis, randomdir.clone()).unwrap();
    let store = InMemState::default();

    let vm = vm::Machine::new(&genesis).unwrap();

    Self {
      genesis,
      store,
      vm,
      keypair,
    }
  }
}

/// Result of a process_transactions call
pub struct ProcessTransactionsResult<D: BlockData> {
  pub block: Produced<D>,
  pub block_output: BlockOutput,
  pub transactions: D,
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

/// Result of processing a single transaction
/// provides easier accessors to get to log, error, and state.
pub struct ProcessTransactionResult<D: BlockData> {
  inner: ProcessTransactionsResult<D>,
}

impl<D: BlockData> ProcessTransactionResult<D> {
  /// returns the resulting logs after processing the transaction
  fn log(&self) -> Option<&Vec<(String, String)>> {
    self.inner.logs().values().next() // as there is only one
  }

  /// returns the resulting errors after executing transactions
  fn error(&self) -> Option<&ContractError> {
    self.inner.errors().values().next()
  }

  /// returns the resulting StateDiff after processing the transactions
  fn state(&self) -> &StateDiff {
    self.inner.state()
  }
}

/// Implements a TestValidator
/// processes a block per transaction.
pub struct TestValidator<'g, D: BlockData> {
  ctx: &'g TestCtx<D>,
  chain: Chain<'g, D, InMemState>,
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

  pub fn get_account(&self, pubkey: Pubkey) -> Option<Account> {
    self.chain.with_head(|s, _| s.get(&pubkey))
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

  pub fn process_transaction(
    &mut self,
    transaction: D,
  ) -> Result<ProcessTransactionResult<D>, MachineError> {
    self
      .process_transactions(transaction)
      .map(|r| ProcessTransactionResult { inner: r })
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

    // TODO: insert blocks for 2 times epoch length
    // two events, one is confirmed and one is finalized

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
    // votes on the block. If its rejected its ignored

    // the only way this can fail is the transaction
    // ordering is wrong. MEV protection
    self.chain.include(produced.clone());

    // we are now not including the produced vote block
    // which will mean we will never finalize blocks. And
    // thus also never storing the state into peristen state.

    // self.chain.include(produced_vote_block); <== enable this to finalize

    // get our expected block out of the forktree
    let block = self
      .chain
      .with_head(|_, b| b.as_any().downcast_ref::<Produced<D>>().cloned())
      .expect("headblock is not a produced, add some more options here?");

    // make sure we got the same block back we produced
    // before. If not, something went wrong on chain.include(produced)
    assert_eq!(produced, block);

    Ok(ProcessTransactionsResult {
      block_output,
      block,
      transactions,
    })
  }
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

    let payer = keypair_default();

    let tx_create = create_pq_token_tx(&payer);

    let result = validator.process_transactions(vec![tx_create]).unwrap();

    assert_eq!(result.logs().values().len(), 1);
    assert_eq!(result.errors().values().len(), 0);
    assert_eq!(
      &result.state().hash().to_b58(),
      "W1biVagiHrDbkdaP2qQ5ktft82yppTfpL1kbGek7xGUiKS"
    );
  }

  #[test]
  fn process_transaction_test() {
    let ctx: TestCtx<Vec<Transaction>> = TestCtx::new();
    let mut validator = TestValidator::new(&ctx);

    let payer = keypair_default();

    let tx_create = create_pq_token_tx(&payer);

    let result = validator.process_transaction(vec![tx_create]).unwrap();

    assert_eq!(result.log().is_some(), true);
    assert_eq!(result.error().is_some(), false);
    assert_eq!(
      &result.state().hash().to_b58(),
      "W1biVagiHrDbkdaP2qQ5ktft82yppTfpL1kbGek7xGUiKS"
    );
  }

  #[test]
  fn add_account_test() {
    let ctx: TestCtx<Vec<Transaction>> = TestCtx::new();
    let validator = TestValidator::new(&ctx);

    let pubkey = Pubkey::unique();
    validator.add_account(pubkey, Account::default());

    let account = validator.get_account(pubkey).unwrap();

    assert_eq!(account, Account::default());
  }

  #[test]
  fn delete_account() {
    let ctx: TestCtx<Vec<Transaction>> = TestCtx::new();
    let validator = TestValidator::new(&ctx);

    let pubkey = Pubkey::unique();
    validator.add_account(pubkey, Account::default());

    assert!(validator.get_account(pubkey).is_some());
    validator.delete_account(pubkey);
    assert!(!validator.get_account(pubkey).is_some());
  }
}
