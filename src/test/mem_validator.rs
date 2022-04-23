use {
  crate::{
    consensus::{
      block::{Block, BlockData, Produced},
      Chain,
      Genesis,
      Vote,
    },
    network::{Network, NetworkCommand},
    primitives::{b58::ToBase58String, Account, Keypair, Pubkey},
    test::{
      currency::create_pq_token_tx,
      in_mem_state::InMemState,
      utils::{genesis_default, keypair_default},
    },
    vm::{
      self,
      BlockOutput,
      ContractError,
      Finalized,
      MachineError,
      State,
      StateDiff,
      StateStore,
      Transaction,
    },
  },
  indexmap::IndexMap,
  libp2p::Multiaddr,
  multihash::Multihash,
  std::{marker::PhantomData, sync::Arc},
  thiserror::Error,
  tokio::sync::mpsc::error::SendError,
};

#[derive(Debug, Error)]
pub enum MemValidatorError<D: BlockData> {
  #[error(transparent)]
  SendError(#[from] SendError<NetworkCommand<D>>),
}

pub type Result<T, D> = std::result::Result<T, MemValidatorError<D>>;

pub struct MemValidator<D: BlockData> {
  genesis: Genesis<D>,
}

impl<D: BlockData> MemValidator<D> {
  pub async fn start(
    genesis: Genesis<D>,
    peers: Vec<Multiaddr>,
    listenaddr: impl Iterator<Item = Multiaddr>,
  ) -> Result<(), D> {
    let keypair = keypair_default();

    // Create the P2P networking layer.
    // Networking runs on its own separate thread,
    // and emits events by calling .poll()

    let mut network = Network::new(
      &genesis,
      crate::network::create_memory_transport(&keypair),
      keypair.clone(),
      listenaddr,
    )
    .await
    .unwrap();

    // connect to bootstrap nodes if specified
    for peer in peers {
      network.connect(peer)?;
    }

    Ok(())
  }
}
