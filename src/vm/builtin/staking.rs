use crate::vm::{
  contract::{self, Environment},
  Machine,
};

/// This builtin contract is responsible for operations around validators
/// joining and leaving the consensus, bonding, slashing and collecting staking
/// rewards.
pub fn contract(
  _env: &Environment,
  _params: &[u8],
  _: &Machine,
) -> contract::Result {
  todo!();
}
