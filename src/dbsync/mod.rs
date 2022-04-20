use {
  crate::{
    consensus::{Block, Vote},
    consumer::{BlockConsumer, Commitment},
    primitives::ToBase58String,
    vm::{Executed, Transaction},
  },
  sqlx::{AnyPool, Connection, Executor},
  tracing::{debug, error, info},
};

/// This type is used to sync updates to the blockchain with an
/// external database. This is used by explorers, analytics, and
/// other systems that need to analyze blockchain data as soon as
/// they become available.
pub struct DatabaseSync {
  pool: AnyPool,
}

impl DatabaseSync {
  pub async fn new(pool: AnyPool) -> Result<Self, sqlx::Error> {
    // on start try to create the database schema if it does not exist.
    // this will also implicitly test the database connection and
    // the connection string.
    info!("DbSync starting...");
    let schema = include_str!("./schemas/0.1.0.sql");
    let mut connection = pool.acquire().await?;
    connection.execute(schema).await?;
    info!("DbSync started successfully");
    Ok(Self { pool })
  }

  async fn sync_block(
    &self,
    block: Executed<Vec<Transaction>>,
    commitment: Commitment,
  ) -> Result<(), sqlx::Error> {
    // non-confirmed blocks are not synced because they
    // may be very soon discarded and may not make it
    // to the canonical chain thus they are irrelevant
    // for history.
    if let Commitment::Included = commitment {
      return Ok(());
    }

    debug!("syncing {} with external database...", *block);

    let mut connection = self.pool.acquire().await?;
    let mut dbtransaction = connection.begin().await?;

    // insert block
    let stmt = block_header_stmt(&block, commitment);
    dbtransaction.execute(stmt.as_str()).await?;

    // data is inserted only on the commited stage, when
    // a block gets finalized, all the data is already in
    // the database and doesn't change. The only thing
    // that changes is the commitment flag on the block.
    if let Commitment::Confirmed = commitment {
      // insert transactions
      for tx in &block.data {
        // first the transaction
        dbtransaction
          .execute(transaction_stmt(&block, tx).as_str())
          .await?;

        // // then its referenced accounts
        // dbtransaction
        //   .execute(transaction_accounts_stmt(tx).as_str())
        //   .await?;

        // // then signatures of signing accounts
        // dbtransaction
        //   .execute(transaction_signers_stmt(tx).as_str())
        //   .await?;

        // // and finally output logs or an error
        // dbtransaction
        //   .execute(transaction_outputs_stmt(tx).as_str())
        //   .await?;
      }

      // insert votes
      for vote in &block.votes {
        dbtransaction
          .execute(vote_stmt(&block, vote).as_str())
          .await?;
      }
    }

    dbtransaction.commit().await?;

    Ok(())
  }
}

#[async_trait::async_trait]
impl BlockConsumer<Vec<Transaction>> for DatabaseSync {
  async fn consume(
    &self,
    block: Executed<Vec<Transaction>>,
    commitment: Commitment,
  ) {
    if let Err(error) = self.sync_block(block, commitment).await {
      error!("dbsync error: {error:?}");
    }
  }
}

/// We're either inserting a block for the first time, or we
/// are finalizing a block.
fn block_header_stmt(
  block: &Executed<Vec<Transaction>>,
  commitment: Commitment,
) -> String {
  match commitment {
    Commitment::Included => unreachable!(),
    Commitment::Confirmed => format!(
      "INSERT INTO block VALUES ({}, '{}', '{}', '{}', '{}', '{}', '{:?}', \
       '{}')",
      block.height,
      block.hash().unwrap().to_b58(),
      block.parent.to_b58(),
      block.signature.0,
      block.signature.1.as_ref().to_b58(),
      block.state_hash().to_b58(),
      commitment,
      chrono::Utc::now().format("%Y-%m-%d %H:%M:%S")
    ),
    Commitment::Finalized => {
      format!(
        "UPDATE block SET commitment = '{:?}' WHERE height = {}",
        commitment, block.height
      )
    }
  }
}

fn transaction_stmt(
  block: &Executed<Vec<Transaction>>,
  tx: &Transaction,
) -> String {
  format!(
    "INSERT INTO transaction VALUES ('{}', {}, '{}', {}, '{}', '{}')",
    tx.hash().to_b58(),
    block.height,
    tx.contract,
    tx.nonce,
    tx.params.to_b58(),
    tx.payer
  )
}

fn transaction_accounts_stmt(_tx: &Transaction) -> String {
  todo!()
}

fn transaction_signers_stmt(_tx: &Transaction) -> String {
  todo!()
}

fn transaction_outputs_stmt(_tx: &Transaction) -> String {
  todo!()
}

fn vote_stmt(block: &Executed<Vec<Transaction>>, vote: &Vote) -> String {
  format!(
    "INSERT INTO vote VALUES ({}, '{}', '{}', '{}', '{}')",
    block.height,
    vote.target.to_b58(),
    vote.justification.to_b58(),
    vote.validator,
    vote.signature.to_b58()
  )
}
