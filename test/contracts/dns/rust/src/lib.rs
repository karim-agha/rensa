use {
  borsh::{BorshDeserialize, BorshSerialize},
  std::{ffi::CString, fmt::Debug, os::raw::c_char},
};

//--------------------------------------------------------------------
// ABI
//--------------------------------------------------------------------

#[link(wasm_import_module = "env")]
extern "C" {
  fn log(message: *const c_char);
  fn abort(error: u64);
}

#[no_mangle]
pub extern "C" fn allocate(size: u32) -> *mut u8 {
  log_message(&format!("will allocate {size} bytes"));
  let mut buf = Vec::with_capacity(size as usize);
  let ptr = buf.as_mut_ptr();
  core::mem::forget(buf);
  ptr
}

#[no_mangle]
pub extern "C" fn environment(ptr: *mut u8, len: usize) -> *const Environment {
  let bytes = unsafe { Vec::from_raw_parts(ptr, len, len) };
  let env = Box::new(Environment::try_from_slice(&bytes[..]).unwrap());
  Box::leak(env)
}

#[no_mangle]
pub extern "C" fn params(ptr: *mut u8, len: usize) -> *const Vec<u8> {
  let bytes = unsafe { Vec::from_raw_parts(ptr, len, len) };
  let env = Box::new(Vec::<u8>::try_from_slice(&bytes[..]).unwrap());
  Box::leak(env)
}

#[no_mangle]
pub extern "C" fn output(obj: &Vec<Output>) -> u64 {
  let bytes = Box::new(obj.try_to_vec().unwrap());
  let len = bytes.len() as u64;
  let addr = Box::leak(bytes).as_ptr() as u64;
  (addr << 32) | len
}

#[no_mangle]
pub extern "C" fn main(
  env: &Environment,
  params: &Vec<u8>,
) -> Box<Vec<Output>> {
  match entrypoint(env, params) {
    Ok(result) => Box::new(result),
    Err(error) => abort_contract(error),
  }
}

//--------------------------------------------------------------------
// SDK
//--------------------------------------------------------------------

#[derive(Clone, BorshSerialize, BorshDeserialize)]
pub struct Pubkey([u8; 32]);

impl Debug for Pubkey {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "Pubkey({})", bs58::encode(self.0).into_string())
  }
}

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct AccountView {
  pub signer: bool,
  pub writable: bool,
  pub executable: bool,
  pub owner: Option<Pubkey>,
  pub data: Option<Vec<u8>>,
}

#[derive(Debug, BorshDeserialize)]
pub struct Environment {
  pub address: Pubkey,
  pub accounts: Vec<(Pubkey, AccountView)>,
}

#[derive(Debug, BorshSerialize)]
pub enum SignatureError {
  InvalidSignature,
  MissingSigners,
}

#[derive(Debug, BorshSerialize)]
pub enum ContractError {
  InvalidTransactionNonce(u64),
  AccountAlreadyExists,
  AccountDoesNotExist,
  TooManyInputAccounts,
  AccountTooLarge,
  LogTooLarge,
  TooManyLogs,
  InvalidInputAccounts,
  InvalidAccountOwner,
  InvalidOutputAccount,
  AccountNotWritable,
  ContractDoesNotExit,
  AccountIsNotExecutable,
  SignatureError(SignatureError),
  InvalidInputParameters,
  UnauthorizedOperation,
  Runtime(String),
  Other(String),
  _ComputationalBudgetExhausted,
}

#[derive(Debug, BorshSerialize)]
pub enum Output {
  LogEntry(String, String),
  CreateOwnedAccount(Pubkey, Option<Vec<u8>>),
  WriteAccountData(Pubkey, Option<Vec<u8>>),
  DeleteOwnedAccount(Pubkey),
  ContractInvoke {
    contract: Pubkey,
    accounts: Vec<(Pubkey, AccountView)>,
    params: Vec<u8>,
  },
  CreateExecutableAccount(Pubkey, Vec<u8>),
}

fn log_message(msg: &str) {
  let msg = CString::new(msg).unwrap();
  unsafe { log(msg.as_ptr()) };
}

fn abort_contract(error: ContractError) -> ! {
  let bytes = Box::new(error.try_to_vec().unwrap());
  let len = bytes.len() as u64;
  let addr = Box::leak(bytes).as_ptr() as u64;
  unsafe { abort((addr << 32) | len) };
  unreachable!();
}

//--------------------------------------------------------------------
// Developer Experience
//--------------------------------------------------------------------

#[derive(Debug, BorshDeserialize)]
pub enum Instruction {
  Register { name: String, owner: Pubkey },
  Update { name: String, owner: Pubkey },
  Release { name: String },
}

fn entrypoint(
  env: &Environment,
  params: &[u8],
) -> Result<Vec<Output>, ContractError> {
  log_message(&format!("environment object: {env:?}"));

  if params.len() == 0 {
    return Err(ContractError::InvalidInputParameters);
  }

  let instruction = Instruction::try_from_slice(&params).unwrap();
  log_message(&format!("instruction: {instruction:?}"));

  if let Instruction::Release { name } = instruction {
    return Err(ContractError::Other(format!(
      "Release is not implemented for {name}"
    )));
  }

  Ok(vec![
    Output::LogEntry("test-key".into(), "test-value".into()),
    Output::CreateOwnedAccount(env.address.clone(), Some(vec![1, 2, 3])),
  ])
}
