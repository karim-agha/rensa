use {
  ed25519_dalek::{PublicKey, SecretKey},
  rand::{prelude::ThreadRng, RngCore},
};

fn main() {
  let mut rng = ThreadRng::default();
  let count: u32 = std::env::args()
    .nth(1)
    .unwrap_or_else(|| "1".to_owned())
    .parse()
    .unwrap();

  for _ in 0..count {
    let mut randbytes = [0u8; 32];
    rng.fill_bytes(&mut randbytes);

    let sk = SecretKey::from_bytes(&randbytes).unwrap();
    let pk: PublicKey = (&sk).into();

    println!("pubkey: {}", bs58::encode(pk.as_bytes()).into_string());
    println!("secret: {}", bs58::encode(sk.as_bytes()).into_string());
    println!();
  }
}
