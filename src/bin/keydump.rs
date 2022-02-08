fn main() {
  let b58string: String = std::env::args().nth(1).unwrap();
  let bytes = bs58::decode(b58string).into_vec().unwrap();
  
  println!("bytes ({}): {:?}", bytes.len(), bytes);
}
