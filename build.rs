fn main() {
  let mut config = prost_build::Config::new();
  config.bytes(&["."]);
  config
    .compile_protos(&["src/network/episub/rpc.proto"], &["src"])
    .unwrap();
}
