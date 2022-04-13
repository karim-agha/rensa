use {
  borsh::BorshDeserialize,
  std::{ffi::CString, fmt::Debug, os::raw::c_char},
};

#[derive(Clone, BorshDeserialize)]
pub struct Pubkey([u8; 32]);

impl Debug for Pubkey {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "Pubkey({})", bs58::encode(self.0).into_string())
  }
}

#[derive(Debug, Clone, BorshDeserialize)]
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

#[derive(Debug, BorshDeserialize)]
pub enum Instruction {
  Register { name: String, owner: Pubkey },
  Update { name: String, owner: Pubkey },
  Release { name: String },
}

#[link(wasm_import_module = "env")]
extern "C" {
  fn log(message: *const c_char);
}

fn log_message(msg: &str) {
  let msg = CString::new(msg).unwrap();
  unsafe { log(msg.as_ptr()) };
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
pub extern "C" fn main(env: &Environment, params: &Vec<u8>) -> u32 {
  log_message(&format!("environment object: {env:?}"));
  let instruction = Instruction::try_from_slice(&params).unwrap();
  log_message(&format!("instruction: {instruction:?}"));

  10
}
