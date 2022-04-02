#[no_mangle]
pub extern "C" fn contract(env: i32) -> i32 {
  env + 18
}
