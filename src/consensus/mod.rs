//! Zamfir, V., et al. "Introducing the minimal CBC Casper family of consensus protocols."
//! Implementation of the Latest Message Driven CBC Casper GHOST consensus

pub mod block;
pub mod chain;
pub mod producer;
pub mod schedule;
pub mod validator;
pub mod vote;

pub mod epoch;
pub mod fault;
mod volatile;

pub trait ToBase58String {
  fn to_b58(&self) -> String;
}

impl<S: multihash::Size> ToBase58String for multihash::MultihashGeneric<S> {
  fn to_b58(&self) -> String {
    bs58::encode(self.to_bytes()).into_string()
  }
}

impl ToBase58String for ed25519_dalek::Signature {
  fn to_b58(&self) -> String {
    bs58::encode(self.to_bytes()).into_string()
  }
}
