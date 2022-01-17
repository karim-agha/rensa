use crate::keys::Pubkey;
use rand::{
  distributions::{WeightedError, WeightedIndex},
  prelude::Distribution,
  SeedableRng,
};
use rand_chacha::ChaCha20Rng;

#[derive(Debug, Clone)]
pub struct Validator {
  pub pubkey: Pubkey,
  pub stake: u128,
}

/// Creates a stake-weighted validator schedule iterator based on
/// a predefined seed function. This iterator will iterate forever
/// returning the next expected validator deterministically for a
/// given seed on all validator instances.
///
/// The source of the enthropy for the seed is not specified here,
/// that is going to be defined in higher level of abstraction.
///
/// So for example to get the leader schedule for an entire epoch
/// consisting of 64 blocks use:
///
/// ```
/// let seed = vec![5u8;32];
/// let validators = Vec<Validator>::new(); // validators with stakes
/// let schedule = ValidatorSchedule::new(seed.try_into()?, &validators)?;
/// 
/// let epoch_validators = schedule.take(64);
/// ```
#[derive(Debug)]
pub struct ValidatorSchedule<'a> {
  rng: ChaCha20Rng,
  dist: WeightedIndex<u128>,
  validators: &'a [Validator],
}

impl<'a> ValidatorSchedule<'a> {
  pub fn new(
    seed: [u8; 32],
    validators: &'a [Validator],
  ) -> Result<Self, WeightedError> {
    Ok(Self {
      rng: ChaCha20Rng::from_seed(seed),
      dist: WeightedIndex::new(validators.iter().map(|v| v.stake))?,
      validators,
    })
  }
}

impl<'a> Iterator for ValidatorSchedule<'a> {
  type Item = &'a Validator;
  fn next(&mut self) -> Option<Self::Item> {
    Some(&self.validators[self.dist.sample(&mut self.rng)])
  }
}
