use {
  super::{
    contract::{
      AccountView,
      ContractEntrypoint,
      ContractError,
      Environment,
      Output,
    },
    output::TransactionOutput,
    Machine,
    State,
    StateDiff,
    Transaction,
  },
  crate::primitives::{Account, Pubkey},
};

/// Represents the execution context of a single transaction.
///
/// This type is responsible for running the transaction logic
/// and then processing all its outputs that affect the external
/// blockchain state. Any failure in the contract logic or in
/// any of the outputs will cause the entire transaction to fail
/// and none of its changes will be persisted.
pub struct ExecutionUnit<'s, 't> {
  entrypoint: ContractEntrypoint,
  env: Environment,
  state: &'s dyn State,
  transaction: &'t Transaction,
}

impl<'s, 't> ExecutionUnit<'s, 't> {
  pub fn new(
    transaction: &'t Transaction,
    state: &'s impl State,
    vm: &Machine,
  ) -> Result<Self, ContractError> {
    // this value is defined in genesis
    if transaction.accounts.len() > vm.max_input_accounts() {
      return Err(ContractError::TooManyInputAccounts);
    }

    // don't proceed unless all tx signatures are valid.
    transaction.verify_signatures()?;

    // for now only builtin contracts are supported, later wasm
    // contracts will be pulled here as well.
    if let Some(entrypoint) = vm.builtin(&transaction.contract).cloned() {
      Ok(Self {
        entrypoint,
        env: Self::create_environment(state, transaction)?,
        state,
        transaction,
      })
    } else {
      Err(ContractError::ContractDoesNotExit)
    }
  }

  /// Consumes the execution unit and returns the state difference
  /// that is caused by running this transaction and all its outputs.
  pub fn execute(self) -> Result<TransactionOutput, ContractError> {
    let entrypoint = self.entrypoint;
    match entrypoint(&self.env, &self.transaction.params) {
      Ok(outputs) => {
        let mut txoutputs = TransactionOutput::default();
        // if the transaction execution successfully ran to
        // completion, then process all its outputs that
        // modify global state. Those outputs may still
        // fail. Any failure in processing returned
        // outputs will revert the entire transaction.
        for output in outputs {
          txoutputs = txoutputs.merge(self.process_output(output)?);
        }
        Ok(txoutputs)
      }
      Err(err) => Err(err),
    }
  }

  /// Creates a self-contained environment object that can be used to
  /// invoke a contract at a given version of the blockchain state.
  fn create_environment(
    state: &impl State,
    transaction: &Transaction,
  ) -> Result<Environment, ContractError> {
    Ok(Environment {
      address: transaction.contract,
      accounts: transaction
        .accounts
        .iter()
        .map(|a| {
          let account_data = state.get(&a.address);
          let mut writable = a.writable;

          if writable {
            // on-curve accounts are writable only if
            // the private key of that account signs
            // the transaction.
            if a.address.has_private_key() {
              if !a.signer {
                writable = false;
              }
            }
            // otherwise, we're dealing with a program owned
            // account, if it already exists, check if
            // it belongs to the called contract.
            //
            // if it does not exist, it may be created by the
            // invoked contract, so it stays writable.
            else if let Some(ref existing) = account_data {
              // executable accounts are never writable
              if existing.executable {
                writable = false;
              }

              // an existing non-executable account that
              // is not on the Ed25519 curve is writable
              // only if its owner is the invoked contract.
              if let Some(ref owner) = existing.owner {
                if owner != &transaction.contract {
                  writable = false;
                }
              } else {
                writable = false;
              }
            }
          }

          let account_view = AccountView {
            signer: a.signer,
            writable,
            executable: account_data
              .as_ref()
              .map(|a| a.executable)
              .unwrap_or(false),
            owner: account_data.as_ref().and_then(|d| d.owner),
            data: account_data.as_ref().and_then(|a| a.data.clone()),
          };
          (a.address, account_view)
        })
        .collect(),
    })
  }

  fn process_output(
    &self,
    output: Output,
  ) -> Result<TransactionOutput, ContractError> {
    match output {
      Output::LogEntry(key, value) => Ok(TransactionOutput {
        state_diff: StateDiff::default(),
        log_entries: vec![(key, value)],
      }),
      Output::CreateOwnedAccount(addr, data) => Ok(TransactionOutput {
        state_diff: self.create_account(addr, data)?,
        ..Default::default()
      }),
      Output::ModifyAccountData(addr, data) => Ok(TransactionOutput {
        state_diff: self.modify_account(addr, data)?,
        ..Default::default()
      }),
    }
  }

  /// Creates a new account owned by the executing contract.
  fn create_account(
    &self,
    address: Pubkey,
    data: Option<Vec<u8>>,
  ) -> Result<StateDiff, ContractError> {
    if self.state.get(&address).is_none() {
      self.set_account(address, Account {
        executable: false,
        owner: Some(self.transaction.contract),
        data,
      })
    } else {
      Err(ContractError::AccountAlreadyExists)
    }
  }

  /// Process an output that modifies an existing
  /// contract owned account.
  fn modify_account(
    &self,
    address: Pubkey,
    data: Option<Vec<u8>>,
  ) -> Result<StateDiff, ContractError> {
    if let Some(acc) = self.state.get(&address) {
      self.set_account(address, Account { data, ..acc })
    } else {
      Err(ContractError::AccountDoesNotExist)
    }
  }

  fn set_account(
    &self,
    address: Pubkey,
    value: Account,
  ) -> Result<StateDiff, ContractError> {
    for (addr, view) in &self.env.accounts {
      if addr == &address {
        if view.writable {
          let mut output = StateDiff::default();
          output.set(address, value).unwrap();
          return Ok(output);
        } else {
          return Err(ContractError::AccountNotWritable);
        }
      }
    }
    Err(ContractError::InvalidOutputAccount)
  }
}
