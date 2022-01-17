use base58::ToBase58;
use ed25519_dalek::{PublicKey, SecretKey};
use rand::{prelude::ThreadRng, RngCore};

fn main() {
  let mut rng = ThreadRng::default();
  let count: u32 = std::env::args()
    .nth(1)
    .unwrap_or("1".to_owned())
    .parse()
    .unwrap();

  for _ in 0..count {
    let mut randbytes = [0u8; 32];
    rng.fill_bytes(&mut randbytes);
    let sk = SecretKey::from_bytes(&randbytes).unwrap();
    let pk: PublicKey = (&sk).into();

    println!("pubkey: {}", pk.as_bytes().to_base58());
    println!("secret: {}", sk.as_bytes().to_base58());
    println!();
  }
}
