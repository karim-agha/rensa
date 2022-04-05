use {
  super::{
    contract::{
      AccountView,
      ContractEntrypoint,
      ContractError,
      Environment,
      NativeContractEntrypoint,
      Output,
    },
    output::TransactionOutput,
    AccountRef,
    Machine,
    State,
    StateDiff,
    Transaction,
    WASM_VM_BUILTIN_ADDR,
  },
  crate::{
    consensus::Limits,
    primitives::{Account, Pubkey},
  },
};

enum Entrypoint {
  Native(NativeContractEntrypoint),
  External(ContractEntrypoint),
}

/// Represents the execution context of a single transaction.
///
/// This type is responsible for running the transaction logic
/// and then processing all its outputs that affect the external
/// blockchain state. Any failure in the contract logic or in
/// any of the outputs will cause the entire transaction to fail
/// and none of its changes will be persisted.
pub struct ExecutionUnit<'s, 't, 'm> {
  vm: &'m Machine,
  limits: &'m Limits,
  entrypoint: Entrypoint,
  env: Environment,
  state: &'s dyn State,
  transaction: &'t Transaction,
}

impl<'s, 't, 'm> ExecutionUnit<'s, 't, 'm> {
  pub fn new(
    transaction: &'t Transaction,
    state: &'s impl State,
    vm: &'m Machine,
  ) -> Result<Self, ContractError> {
    // this value is defined in genesis
    if transaction.accounts.len() > vm.limits().max_input_accounts {
      return Err(ContractError::TooManyInputAccounts);
    }

    // don't proceed unless all tx signatures are valid.
    transaction.verify_signatures()?;

    // construct an execution unit if the contract was found
    if let Some(entrypoint) = vm.builtin(&transaction.contract) {
      Ok(Self {
        entrypoint: Entrypoint::Native(entrypoint),
        limits: vm.limits(),
        env: Self::create_environment(state, transaction)?,
        state,
        transaction,
        vm,
      })
    } else {
      Ok(Self {
        entrypoint: Entrypoint::External(
          vm.contract(&transaction.contract, state)?,
        ),
        limits: vm.limits(),
        env: Self::create_environment(state, transaction)?,
        state,
        transaction,
        vm,
      })
    }
  }

  /// Consumes the execution unit and returns the state difference
  /// that is caused by running this transaction and all its outputs.
  pub fn execute(self) -> Result<TransactionOutput, ContractError> {
    // to prevent transaction reply, the payer account has a
    // nonce field that is incremented with every transaction
    // that it is paying for. Fail the transaction if it is not
    // if the transaction nonce value does not match the current
    // account nonce.
    let payer_nonce = self
      .state
      .get(&self.transaction.payer)
      .map(|a| a.nonce)
      .unwrap_or(0);

    if self.transaction.nonce != payer_nonce {
      return Err(ContractError::InvalidTransactionNonce(payer_nonce));
    }

    let outputs = match self.entrypoint {
      Entrypoint::Native(native) => {
        native(&self.env, &self.transaction.params, self.vm)
      }
      Entrypoint::External(ref external) => {
        external(&self.env, &self.transaction.params)
      }
    };
    match outputs {
      Ok(outputs) => {
        let mut txoutputs = TransactionOutput::default();
        // if the transaction execution successfully ran to
        // completion, then process all its outputs that
        // modify global state. Those outputs may still
        // fail. Any failure in processing returned
        // outputs will revert the entire transaction.
        for output in outputs {
          txoutputs = txoutputs.merge(self.process_output(output)?);

          // ensure its bounded
          if txoutputs.log_entries.len() > self.limits.max_logs_count {
            return Err(ContractError::TooManyLogs);
          }
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
    // creates an isolated copy of an account referenced
    // by a transaction for local processing.
    let create_account_view = |account: &AccountRef| {
      let account_data = state.get(&account.address);
      let mut writable = account.writable;

      if writable {
        // on-curve accounts are writable only if
        // the private key of that account signs
        // the transaction.
        if account.address.has_private_key() {
          if !account.signer {
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
        signer: account.signer,
        writable,
        executable: account_data
          .as_ref()
          .map(|a| a.executable)
          .unwrap_or(false),
        owner: account_data.as_ref().and_then(|d| d.owner),
        data: account_data.as_ref().and_then(|a| a.data.clone()),
      };

      (account.address, account_view)
    };

    Ok(Environment {
      address: transaction.contract,
      accounts: transaction
        .accounts
        .iter()
        .map(create_account_view)
        .collect(),
    })
  }

  fn process_output(
    &self,
    output: Output,
  ) -> Result<TransactionOutput, ContractError> {
    match output {
      Output::LogEntry(key, value) => {
        if key.len() + value.len() > self.limits.max_log_size {
          return Err(ContractError::LogTooLarge);
        }
        Ok(TransactionOutput {
          state_diff: StateDiff::default(),
          log_entries: vec![(key, value)],
        })
      }
      Output::CreateOwnedAccount(addr, data) => {
        if let Some(ref data) = data {
          if data.len() > self.limits.max_account_size {
            return Err(ContractError::AccountTooLarge);
          }
        }
        Ok(TransactionOutput {
          state_diff: self.create_account(addr, data)?,
          ..Default::default()
        })
      }
      Output::WriteAccountData(addr, data) => {
        if let Some(ref data) = data {
          if data.len() > self.limits.max_account_size {
            return Err(ContractError::AccountTooLarge);
          }
        }
        Ok(TransactionOutput {
          state_diff: self.modify_account(addr, data)?,
          ..Default::default()
        })
      }
      Output::DeleteOwnedAccount(addr) => Ok(TransactionOutput {
        state_diff: self.delete_account(addr)?,
        ..Default::default()
      }),
      Output::ContractInvoke {
        contract,
        accounts,
        params,
      } => todo!("invoke {contract:?} {accounts:?} {params:?}"),
      Output::CreateExecutableAccount(address, bytecode) => {
        if bytecode.len() > self.limits.max_contract_size {
          return Err(ContractError::AccountTooLarge);
        }
        Ok(TransactionOutput {
          state_diff: self.create_executable_account(address, bytecode)?,
          ..Default::default()
        })
      }
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
        nonce: 0,
        executable: false,
        owner: Some(self.transaction.contract),
        data,
      })
    } else {
      Err(ContractError::AccountAlreadyExists)
    }
  }

  /// Creates a new executable contract account owned by the executing contract.
  /// This operation is only permitted when emitted by the builtin contract
  /// address: WasmVM1xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx
  fn create_executable_account(
    &self,
    address: Pubkey,
    bytecode: Vec<u8>,
  ) -> Result<StateDiff, ContractError> {
    // special privilage gateway
    if self.env.address != *WASM_VM_BUILTIN_ADDR {
      return Err(ContractError::UnauthorizedOperation);
    }

    if self.state.get(&address).is_none() {
      self.set_account(address, Account {
        nonce: 0,
        executable: true,
        owner: Some(*WASM_VM_BUILTIN_ADDR),
        data: Some(bytecode),
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

  /// Process an output that deletes an existing
  /// contract owned account.
  fn delete_account(
    &self,
    address: Pubkey,
  ) -> Result<StateDiff, ContractError> {
    for (addr, view) in &self.env.accounts {
      if addr == &address {
        if view.owner.is_none() {
          return Err(ContractError::InvalidAccountOwner);
        }

        if view.owner.unwrap() != self.env.address {
          return Err(ContractError::InvalidAccountOwner);
        }

        if !view.writable {
          return Err(ContractError::AccountNotWritable);
        }

        let mut output = StateDiff::default();
        output.remove(address).unwrap();
        return Ok(output);
      }
    }
    Err(ContractError::InvalidOutputAccount)
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
