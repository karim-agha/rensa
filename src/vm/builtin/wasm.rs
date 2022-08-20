//! Wasm VM Contract
//!
//! This builtin contract implements WASM smart contract deployment and update.

use {
  crate::{
    primitives::{Pubkey, ToBase58String},
    vm::{
      contract::{self, AccountView, ContractError, Environment},
      transaction::SignatureError,
      AccountRef,
      Machine,
    },
  },
  borsh::{BorshDeserialize, BorshSerialize},
  serde::Deserialize,
};

type ContractSeed = [u8; 32];
type BytecodeChecksum = [u8; 32];

/// 2kb max per uploaded slot
const MAX_SLOT_SIZE: usize = 2048;

/// This is the instruction param to the wasm deployment contract
#[derive(Debug, Deserialize, BorshSerialize, BorshDeserialize)]
enum Instruction {
  /// Deploys a new WASM smart contract
  ///
  /// Accounts expected by this instruction:
  ///   0. [drw-] Contract destination address [Wasm.derive(seed)]
  ///   1. [drw-] Contract bytecode storage address
  ///       [Wasm.derive(seed, b"bytecode")]
  Allocate {
    /// A seed value used to generate the contract address.
    /// The contract will be deployed at Wasm.derive(seed).
    seed: ContractSeed,

    /// Sha3 of the final WASM bytecode.
    checksum: BytecodeChecksum,

    /// The account that is allowed to upload pieces of the contract code
    authority: Pubkey,

    /// UWASM bytecode size.
    ///
    /// The maximum size of data stored in this vec depends on the
    /// [`max_contract_size`] value in Genesis.
    size: u32,
  },

  /// Uploads a piece of the wasm bytecode and stores it in the bytecode store.
  /// Each chunk is 2kb in size max.
  ///
  /// Accounts expected by this instruction:
  ///   0. [drw-] Contract destination address [Wasm.derive(seed)]
  ///   1. [drw-] Contract bytecode storage address
  ///       [Wasm.derive(seed, b"bytecode")]   
  ///   2. [---s] Signature of the authority account specified during
  ///       [`Allocate`]
  ///
  /// This transaction will fail if it tries to overwrite an already uploaded
  /// slot.
  Upload {
    /// A seed value used to generate the contract address.
    /// The contract will be deployed at Wasm.derive(seed).
    seed: ContractSeed,

    /// Index of the stored bytes.
    ///
    /// Each slot is 2kb except the last slot could be smaller,
    /// so the storage position is [2kb * index, 2kb*index + len]
    index: u16,

    /// A piece of the bytecode at a given position with a maximum length of
    /// 2kb.
    bytes: Vec<u8>,
  },

  /// Once all the contract bytecode is uploaded, this instruction
  /// creates an executable contract out of it and optionally invokes
  /// the contract.
  ///
  /// Accounts expected by this instruction:
  ///   0. [drw-] Contract destination address [Wasm.derive(seed)]
  ///   1. [drw-] Contract bytecode storage address
  ///       [Wasm.derive(seed, b"bytecode")]   
  ///   2. [---s] Signature of the authority account specified during
  ///       [`Allocate`]
  ///   3..N Optional accounts passed to the init instruction
  ///
  /// This instruction will fail if:
  ///   - not all parts of the bytecode were uploaded.
  ///   - The uploaded bytecode is not a valid WASM.
  ///   - The init instruction fails.
  ///
  /// Once the bytecode is installed as an executable, the bytecode
  /// buffer gets deleted and moved to the contract address.
  ///
  /// If this instruction fails because the init instruction failed,
  /// the bytecode will not be deleted and install can be invoked again.
  Install {
    /// A seed value used to generate the contract address.
    /// The contract will be deployed at Wasm.derive(seed).
    seed: ContractSeed,

    /// An optional instruction that is invoked on the contract once
    /// it is deployed as an executable.
    init: Option<Vec<u8>>,
  },
}

/// An account that stores the wasm bytecode while it is being
/// uploaded before it is installed. It's only allowed to be
/// populated and modified by the
#[derive(Debug, Deserialize, BorshSerialize, BorshDeserialize)]
struct BytecodeAccount {
  /// The account that is authorized to upload bytecode, and it must
  /// be the signer of all upload and install transactions.
  authority: Pubkey,

  /// The size of the final wasm contract bytecode.
  size: u32,

  /// Sha3 of the final bytecode contents after its uploaded.
  checksum: BytecodeChecksum,

  /// A bitmask that specifies which 2kb slots of the bytecode
  /// has been already uploaded. Only when all slots were uploaded
  /// the bytecode is allowed to be installed.
  mask: Vec<bool>,

  /// The actual bytecode of the wasm contract, uploaded in chunks
  /// of 2kb. Everytime a chunk is uploaded, its bit is set to 1 in
  /// the mask.
  bytecode: Vec<u8>,
}

/// This builtin contract allows external users of the blockchain to upload
/// and deploy new contracts WASM bytecode.
pub fn contract(
  env: &Environment,
  params: &[u8],
  vm: &Machine,
) -> contract::Result {
  let mut params = params;
  let instruction: Instruction = BorshDeserialize::deserialize(&mut params)
    .map_err(|_| ContractError::InvalidInputParameters)?;

  match instruction {
    Instruction::Allocate {
      seed,
      checksum,
      authority,
      size,
    } => process_allocate(env, seed, checksum, authority, size, vm),
    Instruction::Upload { seed, index, bytes } => {
      process_upload(env, seed, index, bytes)
    }
    Instruction::Install { seed, init } => process_install(env, seed, init, vm),
  }
}

/// Accounts expected by this instruction:
///   0. [dr--] Contract destination address [Wasm.derive(seed)]
///   1. [drw-] Contract bytecode storage address
///       [Wasm.derive(seed, b"bytecode")]
fn process_allocate(
  env: &Environment,
  seed: ContractSeed,
  checksum: BytecodeChecksum,
  authority: Pubkey,
  size: u32,
  vm: &Machine,
) -> contract::Result {
  if env.address.len() != 2 {
    return Err(ContractError::InvalidInputAccounts);
  }

  // ensure that the contract is under the max contract limit
  if size as usize > vm.limits().max_contract_size {
    return Err(ContractError::AccountTooLarge);
  }

  // validate and get the destination account for the contract
  let (c_addr, c_acc) = contract_account(seed, env)?;

  // make sure that this contract address is not already taken, if it is
  // then a different seed value will need to be used.
  if c_acc.executable || c_acc.data.is_some() || c_acc.owner.is_some() {
    return Err(ContractError::AccountAlreadyExists);
  }

  // validate and get the bytecode storage account
  let (b_addr, b_acc) = bytecode_account(seed, env)?;

  // make sure that this contract address is not already taken, if it is
  // then a different seed value will need to be used.
  if b_acc.data.is_some() || b_acc.owner.is_some() {
    return Err(ContractError::AccountAlreadyExists);
  }

  // It will be created as part of this transaction, so it must be writable
  // it is owned by the current contract
  if !b_acc.writable {
    return Err(ContractError::AccountNotWritable);
  }

  // The authorith account will need to sign upload and install transactions,
  // so it must be on the ed25519 curve and cannot be a derived address.
  // Contracts deploying other contracts is not supported at the moment.
  if !authority.has_private_key() {
    return Err(ContractError::InvalidInputAccounts);
  }

  // each 2kb slot takes one bit
  let masklen = f32::ceil(size as f32 / MAX_SLOT_SIZE as f32) as usize;

  // Stores the uploaded wasm chunks until everything is uploaded
  let contents = BytecodeAccount {
    authority,
    size,
    checksum,
    mask: Vec::with_capacity(masklen),
    bytecode: Vec::with_capacity(size as usize),
  };

  Ok(vec![
    contract::Output::LogEntry("action".to_owned(), "allocate".to_owned()),
    contract::Output::LogEntry("contract".to_owned(), c_addr.to_string()),
    contract::Output::LogEntry("size".to_owned(), size.to_string()),
    contract::Output::LogEntry("checksum".to_owned(), checksum.to_b58()),
    contract::Output::LogEntry("authority".to_owned(), authority.to_string()),
    // at this stage only create the bytecode account
    contract::Output::CreateOwnedAccount(
      *b_addr,
      Some(
        contents
          .try_to_vec()
          .map_err(|e| ContractError::Other(e.to_string()))?,
      ),
    ),
  ])
}

/// Accounts expected by this instruction:
///   0. [drw-] Contract destination address [Wasm.derive(seed)]
///   1. [drw-] Contract bytecode storage address
///       [Wasm.derive(seed, b"bytecode")]   
///   2. [---s] Signature of the authority account specified during
///       [`Allocate`]
fn process_upload(
  env: &Environment,
  seed: ContractSeed,
  index: u16,
  bytes: Vec<u8>,
) -> contract::Result {
  if env.address.len() != 3 {
    return Err(ContractError::InvalidInputAccounts);
  }

  if bytes.is_empty() || bytes.len() > MAX_SLOT_SIZE {
    return Err(ContractError::InvalidInputParameters);
  }

  // validate and get the destination account for the contract
  let (c_addr, c_acc) = contract_account(seed, env)?;

  // make sure that this contract address is not already taken, if it is
  // then a different seed value will need to be used.
  if c_acc.executable || c_acc.data.is_some() || c_acc.owner.is_some() {
    return Err(ContractError::AccountAlreadyExists);
  }

  // the bytecode storage account
  let (b_addr, b_acc) = bytecode_account(seed, env)?;

  if !b_acc.writable {
    return Err(ContractError::AccountNotWritable);
  }

  if b_acc.owner.map(|o| o == env.address).unwrap_or(false) {
    return Err(ContractError::InvalidAccountOwner);
  }

  // read the accumulated bytecode content so far
  let mut content: BytecodeAccount = match b_acc.data {
    Some(ref data) => BorshDeserialize::try_from_slice(data.as_slice())
      .map_err(|_| ContractError::InvalidInputAccounts)?,
    None => return Err(ContractError::AccountDoesNotExist),
  };

  // verify authority
  let (a_addr, a_acc) = &env.accounts[2];

  if content.authority != *a_addr {
    return Err(ContractError::InvalidInputAccounts);
  }

  // make sure the uploaded chunk is authorized by the
  // authority that allocated the contract bytecode account.
  if !a_acc.signer {
    return Err(ContractError::SignatureError(
      SignatureError::MissingSigners,
    ));
  }

  // check if the index has already been uploaded
  if content.mask.len() >= index as usize {
    return Err(ContractError::InvalidInputParameters);
  }

  if content.mask[index as usize] {
    return Err(ContractError::Other(format!(
      "slot {} is already uploaded",
      index
    )));
  } else {
    // mark the slot as uploaded
    content.mask[index as usize] = true;

    let start_offset = index as usize * MAX_SLOT_SIZE;
    if start_offset >= content.bytecode.len() {
      return Err(ContractError::InvalidInputParameters);
    }

    let end_offset = start_offset + bytes.len();
    if end_offset >= content.bytecode.len() {
      return Err(ContractError::InvalidInputParameters);
    }

    // merge bytecode bytes
    content.bytecode[start_offset..=end_offset].copy_from_slice(&bytes);
  }

  Ok(vec![
    contract::Output::LogEntry("action".to_owned(), "upload".to_owned()),
    contract::Output::LogEntry("contract".to_owned(), c_addr.to_string()),
    contract::Output::LogEntry("slot".to_owned(), index.to_string()),
    contract::Output::WriteAccountData(
      *b_addr,
      Some(
        content
          .try_to_vec()
          .map_err(|e| ContractError::Other(e.to_string()))?,
      ),
    ),
  ])
}

/// Accounts expected by this instruction:
///   0. [drw-] Contract destination address [Wasm.derive(seed)]
///   1. [drw-] Contract bytecode storage address
///       [Wasm.derive(seed, b"bytecode")]   
///   2. [---s] Signature of the authority account specified during
///       [`Allocate`]
///   3..N Optional accounts passed to the init instruction
fn process_install(
  env: &Environment,
  seed: ContractSeed,
  init: Option<Vec<u8>>,
  _vm: &Machine,
) -> contract::Result {
  if env.address.len() < 3 {
    return Err(ContractError::InvalidInputAccounts);
  }

  // validate and get the destination account for the contract
  let (c_addr, c_acc) = contract_account(seed, env)?;

  if !c_acc.writable {
    return Err(ContractError::AccountNotWritable);
  }

  // the bytecode storage account
  let (b_addr, b_acc) = bytecode_account(seed, env)?;

  if !b_acc.writable {
    return Err(ContractError::AccountNotWritable);
  }

  if b_acc.owner.map(|o| o == env.address).unwrap_or(false) {
    return Err(ContractError::InvalidAccountOwner);
  }

  // read the accumulated bytecode account
  let content: BytecodeAccount = match b_acc.data {
    Some(ref data) => BorshDeserialize::try_from_slice(data.as_slice())
      .map_err(|_| ContractError::InvalidInputAccounts)?,
    None => return Err(ContractError::AccountDoesNotExist),
  };

  // verify authority
  let (a_addr, a_acc) = &env.accounts[2];

  if content.authority != *a_addr {
    return Err(ContractError::InvalidInputAccounts);
  }

  // make sure the install is triggered by the authorized
  // authority that allocated the bytecode account and uploaded
  // the bytecode.
  if !a_acc.signer {
    return Err(ContractError::SignatureError(
      SignatureError::MissingSigners,
    ));
  }

  if content.mask.iter().any(|s| !s) {
    return Err(ContractError::Other(
      "Not all bytecode slots are uploaded".to_owned(),
    ));
  }

  let mut output = vec![
    contract::Output::LogEntry("action".to_owned(), "install".to_owned()),
    contract::Output::LogEntry("contract".to_owned(), c_addr.to_string()),
    contract::Output::DeleteOwnedAccount(*b_addr), // delete bytecode account
    contract::Output::CreateExecutableAccount(*c_addr, content.bytecode),
  ];

  // optionally invoke constructor instruction with the optional
  // accounts passed to the instruction after the first mandatory
  // 3 accounts.
  if let Some(params) = init {
    let accounts = env.accounts[2..].to_vec();
    output.push(contract::Output::ContractInvoke {
      contract: *c_addr,
      accounts: accounts
        .iter()
        .map(|(addr, acc)| AccountRef {
          address: *addr,
          writable: acc.writable,
          signer: acc.signer,
        })
        .collect(),
      params,
    });
  }

  Ok(output)
}

fn contract_account(
  seed: ContractSeed,
  env: &Environment,
) -> Result<(&Pubkey, &AccountView), ContractError> {
  let expected_addr = env.address.derive(&[&seed]);
  if let Some((addr, acc)) = env.accounts.first() {
    if *addr != expected_addr {
      return Err(ContractError::InvalidInputAccounts);
    }
    Ok((addr, acc))
  } else {
    Err(ContractError::InvalidInputAccounts)
  }
}

fn bytecode_account(
  seed: ContractSeed,
  env: &Environment,
) -> Result<(&Pubkey, &AccountView), ContractError> {
  let expected_addr = env.address.derive(&[&seed, b"bytecode"]);
  if let Some((addr, acc)) = env.accounts.get(1) {
    if *addr != expected_addr {
      return Err(ContractError::InvalidInputAccounts);
    }
    Ok((addr, acc))
  } else {
    Err(ContractError::InvalidInputAccounts)
  }
}
