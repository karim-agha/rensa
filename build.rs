
fn main() {
  prost_build::compile_protos(&["src/network/episub/rpc.proto"], &["src"]).unwrap();
}
