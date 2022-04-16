use {
  crate::{ContractError, Environment, Output},
  borsh::{BorshDeserialize, BorshSerialize},
  std::{ffi::CString, os::raw::c_char},
};

#[link(wasm_import_module = "env")]
extern "C" {
  #[link_name = "log"]
  fn abi_log(message: *const c_char);

  #[link_name = "abort"]
  fn abi_abort(error: u64);
}

#[no_mangle]
pub extern "C" fn allocate(size: u32) -> *mut u8 {
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
  let params = Box::new(Vec::<u8>::try_from_slice(&bytes[..]).unwrap());
  Box::leak(params)
}

#[no_mangle]
pub extern "C" fn output(obj: &Vec<Output>) -> u64 {
  let bytes = Box::new(obj.try_to_vec().unwrap());
  let len = bytes.len() as u64;
  let addr = Box::leak(bytes).as_ptr() as u64;
  (addr << 32) | len
}

/// Debug log a message during contract execution.
/// This is a noop when deployed on chain.
pub fn log(msg: &str) {
  let msg = CString::new(msg).unwrap();
  unsafe { abi_log(msg.as_ptr()) };
}

/// Interrupt the execution of a smart contract with an error value.
pub fn abort(error: ContractError) -> ! {
  let bytes = Box::new(error.try_to_vec().unwrap());
  let len = bytes.len() as u64;
  let addr = Box::leak(bytes).as_ptr() as u64;
  unsafe { abi_abort((addr << 32) | len) };
  unreachable!();
}
