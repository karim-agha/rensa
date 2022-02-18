use {
  futures::{
    future::FusedFuture,
    stream::FusedStream,
    Future,
    Stream,
    StreamExt,
  },
  std::{
    pin::Pin,
    task::{Context, Poll},
  },
};

/// Similar to StreamExt Stream.next(), except it works with an
/// option that may contain a stream. In case the option is none
/// then the stream is considered terminated and will always be
/// pending, otherwise it will behave just like the reguler
/// StreamExt::next() trait method.
///
/// This is used in the main() function of the codebase as a more
/// ergonomic way of dealing with optional services that may or
/// may not be turned on, such as the RPC service, the external
/// state sync service, and others.
///
/// This way the surface level apis for core and optional services
/// is unified.
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct OptionNext<'a, S> {
  stream: &'a mut S,
}

impl<S: Unpin> Unpin for OptionNext<'_, S> {}

impl<'a, S: Stream + Unpin> OptionNext<'a, Option<S>> {
  pub(super) fn new(stream: &'a mut Option<S>) -> Self {
    Self { stream }
  }
}

impl<S: FusedStream + Unpin> FusedFuture for OptionNext<'_, Option<S>> {
  fn is_terminated(&self) -> bool {
    self
      .stream
      .as_ref()
      .map(|s| s.is_terminated())
      .unwrap_or(true)
  }
}

impl<S: Stream + Unpin> Future for OptionNext<'_, Option<S>> {
  type Output = Option<S::Item>;

  fn poll(
    mut self: Pin<&mut Self>,
    cx: &mut Context<'_>,
  ) -> Poll<Self::Output> {
    self
      .stream
      .as_mut()
      .map(|s| s.poll_next_unpin(cx))
      .unwrap_or(Poll::Pending)
  }
}

pub trait OptionalStreamExt {
  fn next(&mut self) -> OptionNext<'_, Self>
  where
    Self: Unpin + Sized;
}

impl<S: Stream + Unpin> OptionalStreamExt for Option<S> {
  fn next(&mut self) -> OptionNext<'_, Self>
  where
    Self: Unpin + Sized,
  {
    OptionNext::new(self)
  }
}
