use {
  crate::{
    consensus::{Block, Genesis, Produced, Vote},
    primitives::{Keypair, Pubkey, ToBase58String},
    vm::{self, AccountRef, Executable, State, Transaction},
  },
  borsh::BorshSerialize,
  futures::Stream,
  rand::{distributions::Uniform, thread_rng, Rng},
  rayon::prelude::*,
  std::{
    collections::{HashMap, HashSet, VecDeque},
    mem::take,
    pin::Pin,
    task::{Context, Poll},
  },
  tracing::info,
};

pub struct BlockProducer<'v> {
  keypair: Keypair,
  vm: &'v vm::Machine,
  votes: HashMap<[u8; 64], Vote>,
  mint: Pubkey,
  alternate: bool,
  wallets_a: Vec<Keypair>,
  wallets_b: Vec<Keypair>,
  validators: HashSet<Pubkey>,
  pending: VecDeque<Produced<Vec<Transaction>>>,
}

impl<'v> BlockProducer<'v> {
  pub fn new(
    genesis: &Genesis<Vec<Transaction>>,
    vm: &'v vm::Machine,
    keypair: Keypair,
  ) -> Self {
    let currency_addr: Pubkey = "Currency1xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
      .parse()
      .unwrap();
    let mint_addr = currency_addr.derive(&[&keypair.public()]);

    BlockProducer {
      vm,
      keypair,
      mint: mint_addr,
      alternate: false,
      wallets_a: (0..2500)
        .into_par_iter()
        .map(|_| Keypair::unique())
        .collect(),
      wallets_b: (0..2500)
        .into_par_iter()
        .map(|_| Keypair::unique())
        .collect(),
      votes: HashMap::new(),
      validators: genesis.validators.iter().map(|v| v.pubkey).collect(),
      pending: VecDeque::new(),
    }
  }

  pub fn record_vote(&mut self, vote: Vote) {
    // todo: use BLS aggregate signature to save space and bandwidth
    if self.validators.contains(&vote.validator) {
      self.votes.insert(vote.signature.to_bytes(), vote);
    }
  }

  // remove votes that were already observed in received blocks.
  pub fn exclude_votes(&mut self, block: &Produced<Vec<Transaction>>) {
    for vote in &block.votes {
      self.votes.remove(&vote.signature.to_bytes());
    }
  }

  fn _create_sha_tx(payer: &Keypair) -> Transaction {
    // private key of account CKDN1WjimfErkbgecnEfoPfs7CU1TknwMhpgbiXNknGC
    let signer = "9XhCqH1LxmziWmBb8WnqzuvKFjX7koBuyzwdcFkL1ym7"
      .parse()
      .unwrap();

    Transaction::new(
      "Sha3xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
        .parse()
        .unwrap(),
      payer,
      vec![AccountRef::writable(
        "CKDN1WjimfErkbgecnEfoPfs7CU1TknwMhpgbiXNknGC",
        true,
      )
      .unwrap()],
      b"initial-seed".to_vec(),
      &[&signer],
    )
  }

  fn create_transfer_tx(
    payer: &Keypair,
    mint: &Pubkey,
    from: &Keypair,
    to: &Pubkey,
  ) -> Transaction {
    let currency_addr: Pubkey = "Currency1xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
      .parse()
      .unwrap();

    let from_coin_addr = currency_addr.derive(&[mint, &from.public()]);
    let to_coin_addr = currency_addr.derive(&[mint, to]);

    let dist = Uniform::new(900, 1100);
    let amount = thread_rng().sample(dist);
    Transaction::new(
      currency_addr,
      payer,
      vec![
        AccountRef::readonly(*mint, false).unwrap(),
        AccountRef::readonly(from.public(), true).unwrap(),
        AccountRef::writable(from_coin_addr, false).unwrap(),
        AccountRef::readonly(*to, false).unwrap(),
        AccountRef::writable(to_coin_addr, false).unwrap(),
      ],
      vm::builtin::currency::Instruction::Transfer(amount)
        .try_to_vec()
        .unwrap(),
      &[from],
    )
  }

  /// Creates the coin and then mints 10k coins to
  /// each wallet in group a and group b
  fn create_mint_txs(
    &self,
    payer: &Keypair,
    seed: [u8; 32],
  ) -> Vec<Transaction> {
    let currency_addr: Pubkey = "Currency1xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
      .parse()
      .unwrap();

    let mint_addr = currency_addr.derive(&[&seed]);
    let create_tx = Transaction::new(
      currency_addr,
      payer,
      vec![AccountRef::writable(mint_addr, false).unwrap()],
      vm::builtin::currency::Instruction::Create {
        seed,
        authority: self.keypair.public(),
        decimals: 6,
        name: None,
        symbol: None,
      }
      .try_to_vec()
      .unwrap(),
      &[&self.keypair],
    );

    let mut txs = Vec::with_capacity(5001);

    // create the coin
    txs.push(create_tx);

    // mint 10k coins for each account
    txs.append(
      &mut self
        .wallets_a
        .par_iter()
        .chain(self.wallets_b.par_iter())
        .map(|w| {
          Transaction::new(
            currency_addr,
            payer,
            vec![
              AccountRef::writable(mint_addr, false).unwrap(),
              AccountRef::readonly(self.keypair.public(), true).unwrap(),
              AccountRef::readonly(w.public(), false).unwrap(),
              AccountRef::writable(
                currency_addr.derive(&[&mint_addr, &w.public()]),
                false,
              )
              .unwrap(),
            ],
            vm::builtin::currency::Instruction::Mint(10_000)
              .try_to_vec()
              .unwrap(),
            &[&self.keypair],
          )
        })
        .collect(),
    );

    txs
  }

  pub fn produce(
    &mut self,
    state: &dyn State,
    prev: &dyn Block<Vec<Transaction>>,
  ) {
    let prevhash = prev.hash().unwrap();

    // this account pays the tx costs
    let payer = "6MiU5w4RZVvCDqvmitDqFdU5QMoeS7ywA6cAnSeEFdW"
      .parse()
      .unwrap();

    let txs = if state.get(&self.mint).is_none() {
      let seed = self.keypair.public().to_vec();
      self.create_mint_txs(&payer, seed.try_into().unwrap())
    } else if self.alternate {
      self
        .wallets_a
        .par_iter()
        .zip(self.wallets_b.par_iter())
        .map(|(from, to)| {
          Self::create_transfer_tx(&payer, &self.mint, from, &to.public())
        })
        .collect()
    } else {
      self
        .wallets_b
        .par_iter()
        .zip(self.wallets_a.par_iter())
        .map(|(from, to)| {
          Self::create_transfer_tx(&payer, &self.mint, from, &to.public())
        })
        .collect()
    };

    self.alternate = !self.alternate;

    let statediff = txs.execute(self.vm, state).unwrap();
    let state_hash = statediff.hash();

    let block = Produced::new(
      &self.keypair,
      prev.height() + 1,
      prevhash,
      txs,
      state_hash,
      take(&mut self.votes).into_iter().map(|(_, v)| v).collect(),
    )
    .unwrap();
    info!(
      "Produced {block} on top of {} with {} transactions with state hash: {}",
      prevhash.to_b58(),
      block.data.len(),
      state_hash.to_b58()
    );
    self.pending.push_back(block);
  }
}

impl<'v> Stream for BlockProducer<'v> {
  type Item = Produced<Vec<Transaction>>;

  fn poll_next(
    mut self: Pin<&mut Self>,
    _: &mut Context<'_>,
  ) -> Poll<Option<Self::Item>> {
    if let Some(block) = self.pending.pop_front() {
      return Poll::Ready(Some(block));
    }
    Poll::Pending
  }
}
