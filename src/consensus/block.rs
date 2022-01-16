use serde::Serialize;

/// Represents the type of values on which the consensus protocol
/// decides among many competing versions.
/// 
/// Type parameters:
/// D is type of the underlying data that consensus is trying to 
///   decide on, in case of a blockchain it is going to be Blocks
/// 
/// S is type of the signatures gathered by the conensus to vote
///   and justify blocks in the fork tree.
/// 
pub trait Block<D, S>: Eq + Serialize
where
  D: Eq + Serialize,
  S: Eq + Serialize,
{
  type Hash: Eq;

  /// Hash of this block with its payload.
  fn hash(&self) -> Self::Hash;

  /// The previous block that this block builds
  /// off in the fork tree.
  fn parent(&self) -> Self::Hash;

  /// Block contents, that are opaque to the consensus.
  /// In most cases this is a list of transactions.
  fn data(&self) -> &D;

  /// BLS signature aggregates collected in this block for any
  /// previous blocks in the fork tree.
  fn signatures(&self) -> &[S];
}
