//! Zamfir, V., et al. "Introducing the minimal CBC Casper family of consensus
//! protocols." Implementation of the Latest Message Driven CBC Casper GHOST
//! consensus

mod block;
mod chain;
mod forktree;
mod genesis;
mod orphans;
mod schedule;
mod validator;
mod vote;

pub use {
  block::{Block, BlockData, Produced},
  chain::{Chain, ChainEvent},
  genesis::{Genesis, Limits},
  schedule::{ValidatorSchedule, ValidatorScheduleStream},
  vote::Vote,
};
