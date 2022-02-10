//! Native Currency Contract
//!
//! This builtin contract implements a generic mechanism for working with
//! fungible and non-fungible tokens on the chain.

use {
  crate::{
    primitives::Pubkey,
    vm::{
      contract,
      contract::{ContractError, Environment},
    },
  },
  borsh::{BorshDeserialize, BorshSerialize},
};

/// Represents a single token metadata on the chain.
///
/// The mint account is always owned by the Currency native contract and its
/// address doesn't have a corresponding private key. It can be manipulated
/// only through instructions to the Currency contract.
#[derive(Debug, BorshSerialize, BorshDeserialize)]
pub struct Mint {
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
  pub decimals: u8,

  /// The long version of the token name
  pub name: Option<String>,

  /// The short ticker symbol of the token.
  /// Limited to 9 alphanumeric symbols.
  pub symbol: Option<String>,
}

/// Represents a token account associated with a user wallet.
///
/// Token accounts are never on the Ed25519 curve and will never directly
/// have a corresponding private key, instead all operations are authorized
/// by checking the presence of the signature of the owner account.
///
/// A single wallet may have many token account controlled by the same wallet.
/// Those accounts are generated using this formula:
///
///   TokenAccount = Currency.derive([mint_pubkey,wallet_pubkey])
///
/// The owner of the token acconut is always the currency module
#[derive(Debug, BorshSerialize, BorshDeserialize)]
pub struct TokenAccount {
  /// The token mint associated with this account
  pub mint: Pubkey,

  /// The wallet address that owns tokens in this account.
  pub owner: Pubkey,

  /// The amount of tokens in this account.
  pub balance: u64,
}

/// This is the instruction param to the currency contract
#[derive(Debug, BorshSerialize, BorshDeserialize)]
pub enum Instruction {
  /// Creates new token mint
  ///
  /// Accounts expected by this instruction:
  ///   0. [d-rw] Mint address
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
  ///  0. [d-rw] The mint address
  ///  1. [---s] The mint authority as signer
  ///  2. [----] The recipient wallet owner address
  ///  3. [d-rw] The recipient address (generated through
  /// Currency.derive([mint, wallet])).  2. [-s--] The mint authority as
  /// signer.
  Mint(u64),

  /// Transfers tokens between wallets.
  ///
  /// Must be signed by the private key of the wallet
  /// owner of the sender or the contract that owns the
  /// tokens.
  ///
  /// Accounts expected by this instruction:
  ///  0. [d-r--] The mint address
  ///  1. [---s] The sender wallet owner address as signer
  ///  2. [drw-] The sender token address (generated through
  /// Currency.derive([mint, wallet]))  2. [drw-] The recipient token address
  /// (generated through Currency.derive([mint, wallet]))
  Transfer(u64),

  /// Remove tokens from circulation.
  ///
  /// Must be signed by the private key of the wallet
  /// owner or called by the owning contract.
  ///
  /// Accounts expected by this instruction:
  ///  0. [drw-] The mint address
  ///  1. [---s] The wallet owner address as signer
  ///  2. [drw-] The token address (generated through Currency.derive([mint,
  /// wallet]))
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

pub fn contract(env: Environment, params: &[u8]) -> contract::Result {
  let mut params = params;
  let instruction: Instruction =
    borsh::de::BorshDeserialize::deserialize(&mut params)
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

fn process_create(
  env: Environment,
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
    return Err(ContractError::AccountAlreadyInUse);
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
        borsh::to_vec(&spec)
          .map_err(|_| ContractError::InvalidInputParameters)?,
      ),
    ),
    // generate logs
    contract::Output::LogEntry("action".into(), "mint-created".into()),
    contract::Output::LogEntry("address".into(), addr.to_string()),
  ])
}

fn process_mint(env: Environment, _amount: u64) -> contract::Result {
  if env.accounts.len() != 4 {
    return Err(ContractError::InvalidInputAccounts);
  }

  let (_mint_addr, _mint_data) = &env.accounts[0];
  let (_authority, _) = &env.accounts[1];
  let (_wallet_addr, _) = &env.accounts[2];
  let (_token_acc_addr, _token_acc_data) = &env.accounts[3];

  Ok(vec![])
}

fn process_transfer(_env: Environment, _amount: u64) -> contract::Result {
  todo!();
}

fn process_burn(_env: Environment, _amount: u64) -> contract::Result {
  todo!();
}

fn process_set_authority(
  _env: Environment,
  _authority: Option<Pubkey>,
) -> contract::Result {
  todo!();
}
