use std::{ffi::CString, os::raw::c_char};

#[repr(C)]
struct Region {
  len: u32,
}

pub struct Environemnt {
  val: u32,
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
pub extern "C" fn environment(_ptr: *const u8) -> *const Environemnt {
  let out = Environemnt { val: 18 };
  let outptr = &out as *const Environemnt;
  core::mem::forget(out);
  outptr
}

#[no_mangle]
pub extern "C" fn contract(env: &Environemnt, params: *const u8) -> u32 {
  let region: *const Region = params as *const Region;
  let region: &Region = unsafe { &*region as &_ };
  env.val + 18 + region.len
}
