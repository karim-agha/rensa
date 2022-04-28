use {
  crate::{
    consensus::{Block, Vote},
    consumer::{BlockConsumer, Commitment},
    primitives::{Account, Pubkey, ToBase58String},
    vm::{AccountRef, Executed, Transaction},
  },
  ed25519_dalek::Signature,
  sqlx::{AnyPool, Connection, Executor},
  std::sync::Arc,
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
    block: &Executed<Vec<Transaction>>,
    commitment: Commitment,
  ) -> Result<(), sqlx::Error> {
    // non-confirmed blocks are not synced because they
    // may be very soon discarded and may not make it
    // to the canonical chain thus they are irrelevant
    // for history.
    if let Commitment::Included = commitment {
      return Ok(());
    }

    debug!("syncing {} with external database...", **block);

    let mut connection = self.pool.acquire().await?;
    let mut dbtransaction = connection.begin().await?;

    // insert block
    let stmt = block_header_stmt(block, commitment);
    dbtransaction.execute(stmt.as_str()).await?;

    // data is inserted only on the commited stage, when
    // a block gets finalized, all the data is already in
    // the database and doesn't change. The only thing
    // that changes is the commitment flag on the block.
    if let Commitment::Confirmed = commitment {
      // insert transactions
      for (i, tx) in block.data.iter().enumerate() {
        // first the transaction
        dbtransaction
          .execute(transaction_stmt(block, tx, i).as_str())
          .await?;

        // then its referenced accounts
        for (i, acc) in tx.accounts.iter().enumerate() {
          dbtransaction
            .execute(transaction_account_stmt(tx, acc, i).as_str())
            .await?;
        }

        // then signatures of signing accounts
        for (i, signature) in tx.signatures.iter().enumerate() {
          dbtransaction
            .execute(transaction_signature_stmt(tx, signature, i).as_str())
            .await?;
        }

        // and finally output logs or an error
        if let Some(outputs) = block.output.logs.get(tx.hash()) {
          for (i, output) in outputs.iter().enumerate() {
            dbtransaction
              .execute(transaction_output_stmt(tx, output, i).as_str())
              .await?;
          }
        } else if let Some(error) = block.output.errors.get(tx.hash()) {
          dbtransaction
            .execute(transaction_error_stmt(tx, error.to_string()).as_str())
            .await?;
        }
      }

      // insert votes
      for (i, vote) in block.votes.iter().enumerate() {
        dbtransaction
          .execute(vote_stmt(block, vote, i).as_str())
          .await?;
      }

      // insert state diffs
      for (i, (addr, acc)) in block.output.state.iter().enumerate() {
        dbtransaction
          .execute(state_diff_stmt(block, addr, acc, i).as_str())
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
    block: Arc<Executed<Vec<Transaction>>>,
    commitment: Commitment,
  ) {
    if let Err(error) = self.sync_block(block.as_ref(), commitment).await {
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
  pos: usize,
) -> String {
  format!(
    "INSERT INTO transaction VALUES ('{}', {}, '{}', {}, '{}', '{}', {})",
    tx.hash().to_b58(),
    block.height,
    tx.contract,
    tx.nonce,
    tx.params.to_b58(),
    tx.payer,
    pos
  )
}

fn transaction_account_stmt(
  tx: &Transaction,
  acc: &AccountRef,
  pos: usize,
) -> String {
  format!(
    "INSERT INTO transaction_accounts VALUES ('{}', '{}', {}, {}, {})",
    tx.hash().to_b58(),
    acc.address,
    match acc.signer {
      true => 1,
      false => 0,
    },
    match acc.writable {
      true => 1,
      false => 0,
    },
    pos
  )
}

fn transaction_signature_stmt(
  tx: &Transaction,
  signature: &Signature,
  pos: usize,
) -> String {
  format!(
    "INSERT INTO transaction_signatures VALUES ('{}', '{}', {})",
    tx.hash().to_b58(),
    signature.to_b58(),
    pos
  )
}

fn transaction_output_stmt(
  tx: &Transaction,
  (key, value): &(String, String),
  pos: usize,
) -> String {
  format!(
    "INSERT INTO transaction_logs VALUES ('{}', '{}', '{}', {})",
    tx.hash().to_b58(),
    key,
    value,
    pos
  )
}

fn transaction_error_stmt(tx: &Transaction, error: String) -> String {
  format!(
    "INSERT INTO transaction_errors VALUES ('{}', '{}')",
    tx.hash().to_b58(),
    error
  )
}

fn state_diff_stmt(
  block: &Executed<Vec<Transaction>>,
  account: &Pubkey,
  data: Option<&Account>,
  pos: usize,
) -> String {
  match data {
    Some(acc) => format!(
      "INSERT INTO state_diff VALUES ({}, '{}', {}, {}, {}, {}, 0, {})",
      block.height,
      account,
      match acc.data {
        None => "NULL".to_string(),
        Some(ref bytes) => format!("'{}'", bytes.to_b58()),
      },
      acc.nonce,
      match acc.owner {
        None => "NULL".to_string(),
        Some(ref owner) => format!("'{}'", owner),
      },
      match acc.executable {
        true => 1,
        false => 0,
      },
      pos
    ),
    None => format!(
      "INSERT INTO state_diff VALUES ({}, '{}', NULL, NULL, NULL, 0, 1, {})",
      block.height, account, pos
    ),
  }
}

fn vote_stmt(
  block: &Executed<Vec<Transaction>>,
  vote: &Vote,
  pos: usize,
) -> String {
  format!(
    "INSERT INTO vote VALUES ({}, '{}', '{}', '{}', '{}', {})",
    block.height,
    vote.target.to_b58(),
    vote.justification.to_b58(),
    vote.validator,
    vote.signature.to_b58(),
    pos
  )
}
