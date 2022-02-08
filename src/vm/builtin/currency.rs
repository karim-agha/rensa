//! Native Currency Contract
//!
//! This builtin contract implements a generic mechanism for working with
//! fungible and non-fungible tokens on the chain.

use {
  crate::{
    primitives::Pubkey,
    vm::{contract, contract::Environment},
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

  /// An optional short ticker symbol of the token.
  /// Limited to 9 alphanumeric symbols.
  pub symbol: Option<String>,
}

/// Represents a token account associated with a user wallet.
///
/// Token accounts are never on the Ed25519 curve and will never directly
/// have a corresponding private key, instead all operations are authorized
/// by checking the presence of the signature of the owner account.
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
  /// Creates new token type
  Create(Mint),

  /// Mints new tokens
  ///
  /// Enabled only when the authority is not None and when
  /// the transaction is signed by the authority private key
  /// or invoked by a contract with address equal to the mint
  /// authority.
  Mint(u64),

  /// Transfers tokens between wallets.
  ///
  /// Must be signed by the private key of the wallet
  /// owner of the sender or the contract that owns the 
  /// tokens.
  Transfer(u64),

  /// Remove tokens from circulation.
  ///
  /// Must be signed by the private key of the wallet
  /// owner or called by the owning contract.
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
  SetAuthority(Option<Pubkey>),
}

pub fn contract(_env: Environment, _params: &[u8]) -> contract::Result {
  todo!()
}
