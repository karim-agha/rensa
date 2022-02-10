use {
  super::{
    contract::{
      AccountView,
      ContractEntrypoint,
      ContractError,
      Environment,
      Output,
    },
    Machine,
    State,
    StateDiff,
    Transaction,
  },
  crate::primitives::{Account, Pubkey, ToBase58String},
  tracing::trace,
};

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
    if let Some(entrypoint) = vm.builtin(&transaction.contract).cloned() {
      let env = Self::create_environment(state, transaction)?;
      Ok(Self {
        entrypoint,
        env,
        state,
        transaction,
      })
    } else {
      Err(ContractError::ContractDoesNotExit)
    }
  }

  pub fn execute(self) -> Result<StateDiff, ContractError> {
    let entrypoint = self.entrypoint;
    entrypoint(self.env, &self.transaction.params).map(|outputs| {
      outputs.into_iter().fold(StateDiff::default(), |s, o| {
        s.merge(Self::process_transaction_output(
          o,
          self.state,
          self.transaction,
        ))
      })
    })
  }

  /// Creates a self-contained environment object that can be used to
  /// invoke a contract at a given version of the blockchain state.
  fn create_environment(
    state: &impl State,
    transaction: &Transaction,
  ) -> Result<Environment, ContractError> {
    // don't proceed unless all signatures are ok.
    transaction.verify_signatures()?;

    Ok(Environment {
      address: transaction.contract.clone(),
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
            // invoked contract.
            else if let Some(existing) = account_data {
              // executable contracts are never writable
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
            executable: account_data.map(|a| a.executable).unwrap_or(false),
            owner: account_data.and_then(|d| d.owner.clone()),
            data: account_data.and_then(|a| a.data.clone()),
          };
          (a.address.clone(), account_view)
        })
        .collect(),
    })
  }

  fn process_transaction_output(
    output: Output,
    state: &dyn State,
    transaction: &Transaction,
  ) -> StateDiff {
    let mut txstate = StateDiff::default();
    match output {
      Output::LogEntry(key, value) => {
        trace!(
          "transaction {} log: {key} => {value}",
          transaction.hash().to_b58()
        ); // todo
      }
      Output::ModifyAccountData(addr, data) => {
        trace!(
          "transaction {} modifying account {addr} with {data:?}",
          transaction.hash().to_b58()
        );
        Self::modify_account_data(addr, data, state, &mut txstate, transaction);
      }
      Output::CreateOwnedAccount(addr, data) => {
        trace!(
          "transaction {} creating account {addr} with: {data:?}",
          transaction.hash().to_b58(),
        );
        Self::create_owned_account(
          addr,
          data,
          state,
          &mut txstate,
          transaction,
        );
      }
    };
    txstate
  }

  fn create_owned_account(
    address: Pubkey,
    data: Option<Vec<u8>>,
    state: &dyn State,
    txstate: &mut dyn State,
    transaction: &Transaction,
  ) {
    if state.get(&address).is_none() {
      Self::set_state_if_writable(
        txstate,
        address,
        Account {
          executable: false,
          owner: Some(transaction.contract.clone()),
          data,
        },
        transaction,
      );
    }
  }

  fn modify_account_data(
    address: Pubkey,
    data: Option<Vec<u8>>,
    state: &dyn State,
    txstate: &mut dyn State,
    transaction: &Transaction,
  ) {
    if let Some(acc) = state.get(&address) {
      Self::set_state_if_writable(
        txstate,
        address,
        Account {
          data,
          ..acc.clone()
        },
        transaction,
      );
    }
  }

  fn set_state_if_writable(
    state: &mut dyn State,
    addr: Pubkey,
    value: Account,
    transaction: &Transaction,
  ) {
    for acc in &transaction.accounts {
      if acc.address == addr && acc.writable {
        state.set(addr, value).unwrap();
        break;
      }
    }
  }
}
