//! Zamfir, V., et al. "Introducing the minimal CBC Casper family of consensus
//! protocols." Implementation of the Latest Message Driven CBC Casper GHOST
//! consensus

mod block;
mod chain;
mod forktree;
mod schedule;
mod validator;
mod vote;

pub use block::{Block, BlockData, Genesis, Produced};
pub use chain::{Chain, ChainEvent};
pub use schedule::{ValidatorSchedule, ValidatorScheduleStream};
pub use vote::Vote;
