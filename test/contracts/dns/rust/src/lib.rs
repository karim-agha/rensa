use {
  borsh::BorshDeserialize,
  std::{ffi::CString, os::raw::c_char},
};

#[derive(Debug, Clone, BorshDeserialize)]
pub struct Pubkey([u8; 32]);

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
pub extern "C" fn main(env: &Environment, _params: *const u8, _params_len: u32) -> u32 {
  log_message(&format!("environment object: {env:?}"));
  10
}
