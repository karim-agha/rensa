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
