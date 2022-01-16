/// This is the maximum stake of validators that can fail or exhibit
/// malicious behaviour for the consensus to guarantee finality.
pub const FAULT_TOLERANCE: f32 = 0.32;

/// This is the minimum percentage of stake that needs to vote
/// for the consensus to successfully decide on a block.
pub const FINALITY_THRESHOLD: f32 = 1.0 - FAULT_TOLERANCE;

/// Violations of Casper conensus rules
pub enum ConsensusFault {
  /// This violation can be detected by everyone who receives the message.
  /// The receiver runs the estimator function on the justification of the message,
  /// and checks whether the proposed value is in the set of values returned by the
  /// estimator. This fault does not nessesarily indivate a malicious behavior and
  /// it can be caused by network partition or censorship by other nodes.
  InvalidMessage,

  /// This violation cannot be detected by anyone who receives only one of
  /// the two messages violating this rule. This violation is a type of Byzantine
  /// failure where a validator votes for two distinct forks of the same history.
  ///
  /// The validator then starts maintaining two histories of protocol execution,
  /// one in which only message A is generated, and the other in which only
  /// message B is generated. This indicates a malicious validator and an evidence
  /// of this behaviour attached to a block causes the validator to be slashed for
  /// their entire stake.
  ///
  /// Consensus failure will be caused when a sufficiently large number of
  /// participants engage in this type of Byzantine behavior (over 1/3).
  Equivocation,
}
