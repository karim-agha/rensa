//! Native Currency Contract
//!
//! This builtin contract implements a generic mechanism for working with
//! fungible and non-fungible tokens on the chain.

use {
  crate::{
    primitives::Pubkey,
    vm::{
      contract::{self, AccountView, ContractError, Environment},
      transaction::SignatureError,
    },
  },
  borsh::{BorshDeserialize, BorshSerialize},
  serde::Deserialize,
};

/// Represents a single token metadata on the chain.
///
/// The mint account is always owned by the Currency native contract and its
/// address doesn't have a corresponding private key. It can be manipulated
/// only through instructions to the Currency contract.
#[derive(Debug, Deserialize, BorshSerialize, BorshDeserialize)]
struct Mint {
  /// Optional authority specifies the pubkey that is allowed to mint
  /// new tokens for this token. If set to None, then no more tokens
  /// of this type can be ever minted.
  pub authority: Option<Pubkey>,

  /// The total supply of tokens.
  ///
  /// For NFTs this is always 1, for regular tokens this represents
  /// the total number of all tokens in all accounts.
  pub supply: u64,

  /// The number of base 10 digits to the right of the decimal place.
  /// For NFTs this is always 0.
  pub decimals: u8,

  /// The long version of the token name
  pub name: Option<String>,

  /// The short ticker symbol of the token.
  /// Limited to 9 alphanumeric symbols.
  pub symbol: Option<String>,
}

/// Represents a Coin account associated with a user wallet.
///
/// Coin accounts are never on the Ed25519 curve and will never directly
/// have a corresponding private key, instead all operations are authorized
/// by checking the presence of the signature of the owner account.
///
/// A single wallet may have many token account controlled by the same wallet.
/// Those accounts are generated using this formula:
///
///   CoinAccount = Currency.derive([mint_pubkey,wallet_pubkey])
///
/// The owner of the token acconut is always the currency module
#[derive(Debug, Deserialize, BorshSerialize, BorshDeserialize)]
struct CoinAccount {
  /// The token mint associated with this account
  pub mint: Pubkey,

  /// The wallet address that owns tokens in this account.
  pub owner: Pubkey,

  /// The amount of tokens in this account.
  pub balance: u64,
}

/// This is the instruction param to the currency contract
#[derive(Debug, BorshSerialize, BorshDeserialize)]
enum Instruction {
  /// Creates new token mint
  ///
  /// Accounts expected by this instruction:
  ///   0. [drw-] Mint address
  Create {
    /// A unique seed that is used to generate the mint address
    /// for an account. The mit address account will be the result
    /// of running: `Pubkey(Currency).derive(seed)`. it must be a
    /// non-existing account, the currencly module will create it
    /// and configure it according to the spec.   
    seed: [u8; 32],

    /// The initial creation of a mint must have a mint authority
    /// otherwise no tokens will ever be minted.
    authority: Pubkey,

    /// The number of base 10 digits to the right of the decimal place.
    decimals: u8,

    /// The long version of the token name (64 bytes max)
    name: Option<String>,

    /// The short ticker symbol of the token.
    /// between 1-9 bytes.
    symbol: Option<String>,
  },

  /// Mints new tokens
  ///
  /// Enabled only when the authority is not None and when
  /// the transaction is signed by the authority private key
  /// or invoked by a contract with address equal to the mint
  /// authority.
  ///
  /// Accounts expected by this instruction:
  ///  0. [drw-] The mint address
  ///  1. [---s] The mint authority as signer
  ///  2. [----] The recipient wallet owner address
  ///  3. [drw-] The recipient address (Currency.derive([mint, wallet])))
  Mint(u64),

  /// Transfers tokens between wallets.
  ///
  /// Must be signed by the private key of the wallet
  /// owner of the sender or the contract that owns the
  /// tokens.
  ///
  /// Accounts expected by this instruction:
  //  0. [d-r--] The mint address
  //  1. [---s] The sender wallet owner address as signer
  //  2. [drw-] The sender coin address
  //  3. [----] The recipient wallet owner address
  //  4. [drw-] The recipient token address
  Transfer(u64),

  /// Remove tokens from circulation.
  ///
  /// Must be signed by the private key of the wallet
  /// owner or called by the owning contract.
  ///
  /// Accounts expected by this instruction:
  ///  0. [drw-] The mint address
  ///  1. [---s] The wallet owner address as signer
  ///  2. [drw-] The coin account address
  Burn(u64),

  /// Changes the mint authority of the token.
  ///
  /// The transaction invoking this instruction must be
  /// signed by the private key of the current authority
  /// or invoked by the contract with address equal to the
  /// current authority.
  ///
  /// Setting the authority to None is an irreversible operation
  /// and forever foregoes the ability to mint new tokens of this
  /// type.
  ///
  /// Accounts expected by this instruction:
  ///  0. [drw-] The mint address
  ///  1. [---s] The the current authority wallet as signer
  SetAuthority(Option<Pubkey>),
}

pub fn contract(env: &Environment, params: &[u8]) -> contract::Result {
  let mut params = params;
  let instruction: Instruction = BorshDeserialize::deserialize(&mut params)
    .map_err(|_| ContractError::InvalidInputParameters)?;

  match instruction {
    Instruction::Create {
      seed,
      authority,
      decimals,
      name,
      symbol,
    } => process_create(env, &seed, authority, decimals, name, symbol),
    Instruction::Mint(amount) => process_mint(env, amount),
    Instruction::Transfer(amount) => process_transfer(env, amount),
    Instruction::Burn(amount) => process_burn(env, amount),
    Instruction::SetAuthority(account) => process_set_authority(env, account),
  }
}

/// Creates new Currency coin and allocates
/// its mint account.
fn process_create(
  env: &Environment,
  seed: &[u8],
  authority: Pubkey,
  decimals: u8,
  name: Option<String>,
  symbol: Option<String>,
) -> contract::Result {
  if env.accounts.len() != 1 {
    return Err(ContractError::InvalidInputAccounts);
  }

  let (addr, value) = &env.accounts[0];

  // check if the mint address in the accounts list
  // is correctly derived from the seed and the currency
  // module pubkey
  let expected_mint_address = env.address.derive(&[seed]);
  if &expected_mint_address != addr {
    return Err(ContractError::InvalidInputAccounts);
  }

  // is this already in use by some other mint?
  if value.data.is_some() || value.owner.is_some() {
    return Err(ContractError::AccountAlreadyExists);
  }

  // validate mint specs
  if let Some(ref symbol) = symbol {
    if symbol.is_empty() || symbol.len() > 10 {
      return Err(ContractError::InvalidInputParameters);
    }
  }

  if let Some(ref name) = name {
    if name.is_empty() || name.len() > 64 {
      return Err(ContractError::InvalidInputParameters);
    }
  }

  if decimals > 20 {
    // won't fit in u64
    return Err(ContractError::InvalidInputParameters);
  }

  let spec = Mint {
    authority: Some(authority),
    supply: 0,
    decimals,
    name,
    symbol,
  };

  Ok(vec![
    // initialize the mint account
    contract::Output::CreateOwnedAccount(
      expected_mint_address,
      Some(
        spec
          .try_to_vec()
          .map_err(|_| ContractError::InvalidInputParameters)?,
      ),
    ),
    // generate logs
    contract::Output::LogEntry("action".into(), "create".into()),
    contract::Output::LogEntry("address".into(), addr.to_string()),
    contract::Output::LogEntry("name".into(), spec.name.unwrap_or_default()),
    contract::Output::LogEntry(
      "symbol".into(),
      spec.symbol.unwrap_or_default(),
    ),
    contract::Output::LogEntry("decimals".into(), decimals.to_string()),
  ])
}

/// Mints new tokens to a specific wallet associated token account
/// and increases the total token supply by the given amount.
fn process_mint(env: &Environment, amount: u64) -> contract::Result {
  if env.accounts.len() != 4 {
    return Err(ContractError::InvalidInputAccounts);
  }

  if amount == 0 {
    return Err(ContractError::InvalidInputParameters);
  }

  //  0. [d-rw] The mint address
  //  1. [---s] The mint authority as signer
  //  2. [----] The recipient wallet owner address
  //  3. [d-rw] The recipient address (coin address)
  let (mint_addr, mint_data) = &env.accounts[0];
  let (authority, authority_acc) = &env.accounts[1];
  let (wallet_addr, _) = &env.accounts[2];
  let (coin_addr, coin_acc) = &env.accounts[3];

  // validate and read coin mint data
  let mut mint = read_coin_mint(mint_addr, mint_data, env)?;

  // make sure that the caller is authorized to mint new tokens
  if let Some(ref mint_authority) = mint.authority {
    // fail if accounts do not match
    if mint_authority != authority {
      return Err(ContractError::Other(
        "account not authorized to mint new coins of this currency".to_owned(),
      ));
    }

    // fail if the authority is not a signer of the transaction
    if !authority_acc.signer {
      return Err(ContractError::SignatureError(
        SignatureError::MissingSigners,
      ));
    }
  } else {
    return Err(ContractError::Other(
      "Minting new coins is disabled for this currency".to_owned(),
    ));
  }

  // logs for explorers and dApps
  let mut outputs = vec![
    contract::Output::LogEntry("action".into(), "mint".into()),
    contract::Output::LogEntry("to".into(), wallet_addr.to_string()),
    contract::Output::LogEntry("amount".into(), amount.to_string()),
  ];

  // all checks passed, now the coin account either has to be created because
  // this is the first time this wallet is used for this coin, or modified
  // by increasing its balance of this coin
  match read_coin_account(coin_addr, coin_acc, mint_addr, wallet_addr, env)? {
    Some(mut coin) => {
      coin.balance = coin.balance.saturating_add(amount);
      outputs.push(contract::Output::ModifyAccountData(
        *coin_addr,
        Some(coin.try_to_vec()?),
      ))
    }
    None => {
      // never used before for this coin, create
      let coin = CoinAccount {
        mint: *mint_addr,
        balance: amount,
        owner: *wallet_addr,
      };
      outputs.push(contract::Output::CreateOwnedAccount(
        *coin_addr,
        Some(coin.try_to_vec()?),
      ));
    }
  };

  // update the global supply value
  mint.supply = mint.supply.saturating_add(amount);
  outputs.push(contract::Output::ModifyAccountData(
    *mint_addr,
    Some(mint.try_to_vec()?),
  ));

  Ok(outputs)
}

fn process_transfer(env: &Environment, amount: u64) -> contract::Result {
  if env.accounts.len() != 5 {
    return Err(ContractError::InvalidInputAccounts);
  }

  if amount == 0 {
    return Err(ContractError::InvalidInputParameters);
  }

  //  0. [d-r--] The mint address
  //  1. [---s] The sender wallet owner address as signer
  //  2. [drw-] The sender coin address
  //  3. [----] The recipient wallet owner address
  //  4. [drw-] The recipient token address
  let (mint_addr, mint_acc) = &env.accounts[0];
  let (sender_wallet_addr, sender_wallet_acc) = &env.accounts[1];
  let (sender_coin_addr, sender_coin_acc) = &env.accounts[2];
  let (recipient_wallet_addr, _) = &env.accounts[3];
  let (recipient_coin_addr, recipient_coin_acc) = &env.accounts[4];

  // make sure that the mint address points to a valid coin mint.
  read_coin_mint(mint_addr, mint_acc, env)?;

  // validate and read coin mint data, this also validates
  // the validity of the derived coin address.
  if let Some(mut sender_coin) = read_coin_account(
    sender_coin_addr,
    sender_coin_acc,
    mint_addr,
    sender_wallet_addr,
    env,
  )? {
    // make sure that the transafer is authorized
    // by the coin wallet account owner
    if !sender_wallet_acc.signer {
      return Err(ContractError::SignatureError(
        SignatureError::MissingSigners,
      ));
    }

    // make sure that the sender has enough balance
    if sender_coin.balance < amount {
      return Err(ContractError::Other(
        "Not enough balance in sender's account".to_owned(),
      ));
    }

    // all checks passed, first debit the sender account
    sender_coin.balance = sender_coin.balance.saturating_sub(amount);

    let mut outputs = vec![
      // logs for explorers and dApps
      contract::Output::LogEntry("action".into(), "transfer".into()),
      contract::Output::LogEntry("from".into(), sender_wallet_addr.to_string()),
      contract::Output::LogEntry("coin".into(), mint_addr.to_string()),
      contract::Output::LogEntry(
        "to".into(),
        recipient_wallet_addr.to_string(),
      ),
      contract::Output::LogEntry("amount".into(), amount.to_string()),
      // store updated debited sender account
      contract::Output::ModifyAccountData(
        *sender_coin_addr,
        Some(sender_coin.try_to_vec()?),
      ),
    ];

    match read_coin_account(
      recipient_coin_addr,
      recipient_coin_acc,
      mint_addr,
      recipient_wallet_addr,
      env,
    )? {
      // the recipient already has a coin account for this coin type
      Some(mut recipient_coin) => {
        recipient_coin.balance = recipient_coin.balance.saturating_add(amount);
        outputs.push(contract::Output::ModifyAccountData(
          *recipient_coin_addr,
          Some(recipient_coin.try_to_vec()?),
        ));
      }
      None => {
        // this is the first time a coin of this type is
        // transferred to this wallet, create a coin account
        let coin = CoinAccount {
          mint: *mint_addr,
          balance: amount,
          owner: *recipient_wallet_addr,
        };

        // create coin account for recipient wallet
        outputs.push(contract::Output::CreateOwnedAccount(
          *recipient_coin_addr,
          Some(coin.try_to_vec()?),
        ));
      }
    }

    if sender_coin.balance == 0 {
      // collect dust coin account, that have zero balance after the
      // transaction.
      outputs.push(contract::Output::DeleteOwnedAccount(*sender_coin_addr));
    }

    // success, coins transfer completed
    Ok(outputs)
  } else {
    Err(ContractError::AccountDoesNotExist)
  }
}

fn process_burn(env: &Environment, amount: u64) -> contract::Result {
  if env.accounts.len() != 3 {
    return Err(ContractError::InvalidInputAccounts);
  }

  if amount == 0 {
    return Err(ContractError::InvalidInputParameters);
  }

  // Accounts expected by this instruction:
  //  0. [drw-] The mint address
  //  1. [---s] The wallet owner address as signer
  //  2. [drw-] The coin account address
  let (mint_addr, mint_acc) = &env.accounts[0];
  let (wallet_addr, wallet_acc) = &env.accounts[1];
  let (coin_addr, coin_acc) = &env.accounts[2];

  let mut mint = read_coin_mint(mint_addr, mint_acc, env)?;
  if let Some(mut coin) =
    read_coin_account(coin_addr, coin_acc, mint_addr, wallet_addr, env)?
  {
    // make sure the owning wallet is authorizing this burn
    if !wallet_acc.signer {
      return Err(ContractError::SignatureError(
        SignatureError::MissingSigners,
      ));
    }

    // make sure the wallet coin account owns enough coins to burn
    if coin.balance < amount {
      return Err(ContractError::Other(
        "Not enough balance in sender's account".to_owned(),
      ));
    }

    // all good, decrement balances on the account and the total coin supply
    mint.supply = mint.supply.saturating_sub(amount);
    coin.balance = coin.balance.saturating_sub(amount);

    let mut outputs = vec![
      // logs for explorers and dApps
      contract::Output::LogEntry("action".into(), "burn".into()),
      contract::Output::LogEntry("from".into(), wallet_addr.to_string()),
      contract::Output::LogEntry("amount".into(), amount.to_string()),
      contract::Output::LogEntry("coin".into(), mint_addr.to_string()),
      // store updated accounts
      contract::Output::ModifyAccountData(*mint_addr, Some(mint.try_to_vec()?)),
      contract::Output::ModifyAccountData(*coin_addr, Some(coin.try_to_vec()?)),
    ];

    if coin.balance == 0 {
      // collect dust coin account, that have zero balance after the
      // transaction.
      outputs.push(contract::Output::DeleteOwnedAccount(*coin_addr));
    }

    Ok(outputs)
  } else {
    Err(ContractError::AccountDoesNotExist)
  }
}

fn process_set_authority(
  env: &Environment,
  authority: Option<Pubkey>,
) -> contract::Result {
  if env.accounts.len() != 2 {
    return Err(ContractError::InvalidInputAccounts);
  }

  // Accounts expected by this instruction:
  //  0. [drw-] The mint address
  //  1. [---s] The the current authority wallet as signer
  let (mint_addr, mint_acc) = &env.accounts[0];
  let (current_auth_addr, current_auth_acc) = &env.accounts[1];

  let mut mint = read_coin_mint(mint_addr, mint_acc, env)?;

  if let Some(ref existing_authority) = mint.authority {
    if existing_authority != current_auth_addr {
      return Err(ContractError::Other(
        "account not authorized to change mint authority".to_owned(),
      ));
    }

    // make sure that the current authority is authorizing this change
    if !current_auth_acc.signer {
      return Err(ContractError::SignatureError(
        SignatureError::MissingSigners,
      ));
    }

    // update authority on mint metadata
    mint.authority = authority;

    let coin_name = mint.name.clone().unwrap_or_default();
    let coin_symbol = mint.symbol.clone().unwrap_or_default();

    Ok(vec![
      // logs for dApps and explorers
      contract::Output::LogEntry("action".into(), "set-authority".into()),
      contract::Output::LogEntry("coin.address".into(), mint_addr.to_string()),
      contract::Output::LogEntry("coin.name".into(), coin_name),
      contract::Output::LogEntry("coin.symbol".into(), coin_symbol),
      contract::Output::LogEntry(
        "to".into(),
        authority.map(|a| a.to_string()).unwrap_or_default(),
      ),
      // update mint storage value
      contract::Output::ModifyAccountData(*mint_addr, Some(mint.try_to_vec()?)),
    ])
  } else {
    Err(ContractError::Other(
      "Authority has been removed for this coin".to_owned(),
    ))
  }
}

/// Verifies that the given account is a valid
/// coin mint account and returns its deserialized
/// representation.
fn read_coin_mint(
  addr: &Pubkey,
  acc: &AccountView,
  env: &Environment,
) -> Result<Mint, ContractError> {
  // the mint account must be a derived account
  if addr.has_private_key() {
    return Err(ContractError::InvalidInputAccounts);
  }

  // the mint account must be owned by the currency contract
  match acc.owner {
    Some(ref owner) => {
      if owner != &env.address {
        return Err(ContractError::InvalidAccountOwner);
      }
    }
    None => {
      return Err(ContractError::InvalidAccountOwner);
    }
  }

  // get the mint account data
  match acc.data {
    Some(ref data) => BorshDeserialize::try_from_slice(data)
      .map_err(|_| ContractError::InvalidInputAccounts),
    None => Err(ContractError::InvalidInputAccounts),
  }
}

/// Verifies that the coin account is of the same type
/// as the specified mint and returns the account data
/// if the coin account exists, otherwise if the wallet
/// was never used before for this coin, None is returned
fn read_coin_account(
  coin_addr: &Pubkey,
  coin_acc: &AccountView,
  mint_addr: &Pubkey,
  wallet_addr: &Pubkey,
  env: &Environment,
) -> Result<Option<CoinAccount>, ContractError> {
  // verify that the coin account is the expected
  // currency account for the given wallet
  if coin_addr != &env.address.derive(&[mint_addr, wallet_addr]) {
    return Err(ContractError::InvalidInputAccounts);
  }

  // verify that it is owned by the currency contract
  // if the account already exists
  if let Some(ref owner) = coin_acc.owner {
    if owner != &env.address {
      return Err(ContractError::InvalidAccountOwner);
    }
  }

  if let Some(ref data) = coin_acc.data {
    // already exists, read its contents
    let coin: CoinAccount = BorshDeserialize::try_from_slice(data)
      .map_err(|_| ContractError::InvalidInputAccounts)?;

    // make sure that the coin account is for the minted coin type
    if mint_addr != &coin.mint {
      return Err(ContractError::InvalidInputAccounts);
    }

    if wallet_addr != &coin.owner {
      return Err(ContractError::InvalidAccountOwner);
    }

    Ok(Some(coin))
  } else {
    Ok(None)
  }
}
