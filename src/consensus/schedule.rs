use {
  super::{validator::Validator, BlockData, Genesis},
  chrono::{DateTime, Utc},
  futures::Stream,
  rand::{
    distributions::{WeightedError, WeightedIndex},
    prelude::Distribution,
    SeedableRng,
  },
  rand_chacha::ChaCha20Rng,
  std::{
    iter::Enumerate,
    pin::Pin,
    task::{Context, Poll, Waker},
    time::Duration,
  },
  tokio::{sync::watch, time::Instant},
};

/// Creates a stake-weighted validator schedule iterator based on
/// a predefined seed value. This iterator will iterate forever
/// returning the next expected validator deterministically for a
/// given seed on all validator instances.
///
/// The source of the entropy for the seed is not specified here,
/// that is going to be defined in higher level of abstraction.
///
/// So for example to get the leader schedule for an entire epoch
/// consisting of 64 blocks use:
///
/// ```
/// let seed = vec![5u8;32];
/// let validators = Vec<Validator>::new(); // validators with stakes
/// let schedule = ValidatorSchedule::new(seed.try_into()?, &validators)?;
///
/// let epoch_validators = schedule.take(64);
/// ```
#[derive(Debug)]
pub struct ValidatorSchedule<'g, D: BlockData> {
  rng: ChaCha20Rng,
  dist: WeightedIndex<u64>,
  genesis: &'g Genesis<D>,
}

impl<'g, D: BlockData> ValidatorSchedule<'g, D> {
  pub fn new(
    seed: [u8; 32],
    genesis: &'g Genesis<D>,
  ) -> Result<Self, WeightedError> {
    Ok(Self {
      rng: ChaCha20Rng::from_seed(seed),
      dist: WeightedIndex::new(Self::validators(genesis).map(|v| v.stake))?,
      genesis,
    })
  }

  fn validators(genesis: &'g Genesis<D>) -> impl Iterator<Item = &Validator> {
    genesis
      .validators
      .iter()
      .filter(|v| v.stake >= genesis.limits.minimum_stake)
  }
}

impl<'g, D: BlockData> Iterator for ValidatorSchedule<'g, D> {
  type Item = &'g Validator;

  fn next(&mut self) -> Option<Self::Item> {
    Self::validators(self.genesis).nth(self.dist.sample(&mut self.rng))
  }
}

/// Synchronizes a validator schedule with the system time
/// and emits events whenever new slot begins. The general
/// expectation is that validators clocks are synchronized
/// through external means such as global NTP servers and
/// they are not different more than a small fraction of
/// one slot time, otherwise we will have multiple validators
/// thinking that it is their turn at the same time.
///
/// Example:
///
/// ```
/// let seed = [5u8; 32];
/// let validators = &genesis.validators;
///
/// let mut schedule = ValidatorSchedule::new(seed, &validators)?;
/// let mut schedule_stream = ValidatorScheduleStream::new(
///   &mut schedule,
///   genesis.genesis_time,
///   genesis.slot_interval,
/// );
///
/// while let Some((slot, validator)) = schedule_stream.next().await {
///   info!("I think that slot {slot} is for: {validator:?}");
/// }
/// ```
pub struct ValidatorScheduleStream<'g, D: BlockData> {
  pos: u64,
  waker: watch::Sender<Option<Waker>>,
  notif: watch::Receiver<u64>,
  schedule: Enumerate<ValidatorSchedule<'g, D>>,
}

impl<'g, D: BlockData> ValidatorScheduleStream<'g, D> {
  pub fn new(
    schedule: ValidatorSchedule<'g, D>,
    genesis: DateTime<Utc>,
    slot: Duration,
  ) -> Self {
    let (tx, rx) = watch::channel(0);
    let (waker_tx, waker_rx) = watch::channel::<Option<Waker>>(None);

    tokio::spawn(async move {
      // if the blockchain hasn't started yet, wait for it
      // and wake this future again on the date of the
      // genesis.
      if Utc::now() < genesis {
        let wait_for = Utc::now() - genesis;
        tokio::time::sleep(Duration::from_millis(
          wait_for.num_milliseconds() as u64
        ))
        .await;
      }

      // next block slot time start
      let mut next_at = Instant::now();

      // how much time passed since the genesis
      let elapsed = Utc::now() - genesis;

      // what is the current slot height we're at
      let slots = elapsed.num_milliseconds() as u64 / slot.as_millis() as u64;

      // what is the time of the start of the next slot
      let next = (slots + 1) * slot.as_millis() as u64;

      // how much time we have left in this slot
      let rem = next - elapsed.num_milliseconds() as u64;
      next_at += Duration::from_millis(rem);

      // the current slot height
      let mut pos = slots;

      // wait until the end of this slot time to start signalling at
      // an aligned time of the start of a slot.
      tokio::time::sleep_until(next_at).await;

      // and now signal all slot increases every slot time
      loop {
        tx.send(pos).unwrap();
        next_at += slot;
        tokio::time::sleep_until(next_at).await;
        let waker = &*waker_rx.borrow();
        if let Some(waker) = waker {
          waker.wake_by_ref();
        }
        pos += 1;
      }
    });

    Self {
      pos: 0,
      waker: waker_tx,
      notif: rx,
      schedule: schedule.enumerate(),
    }
  }
}

impl<'g, D: BlockData> Stream for ValidatorScheduleStream<'g, D> {
  type Item = (u64, &'g Validator);

  // (slot#, validator)

  fn poll_next(
    mut self: Pin<&mut Self>,
    cx: &mut Context<'_>,
  ) -> Poll<Option<Self::Item>> {
    let scheduler_val = *self.notif.borrow();

    // if the latest yielded value is behind the
    // current slot, catch up and return the validator
    // for the current slot height.
    if self.pos < scheduler_val {
      for turn in self.schedule.by_ref() {
        let generator_turn = turn.0 as u64;

        if generator_turn < scheduler_val {
          // the validation schedule needs to catch
          // up with the current slot height.
          continue;
        }

        if generator_turn == scheduler_val {
          // generator is caught up with the current slot number, return
          self.pos = generator_turn;

          // this waker is used to poll this stream future again
          // when a new slot value is available in the bg task.
          self.waker.send(Some(cx.waker().clone())).unwrap();
          return Poll::Ready(Some((generator_turn, turn.1)));
        }

        unreachable!();
      }
    }

    self.waker.send(Some(cx.waker().clone())).unwrap();
    Poll::Pending
  }
}
