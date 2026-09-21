//! A call's reply channel, for a caller that has no executor.
//!
//! The async backend hands the caller a [`futures_channel::oneshot::Receiver`],
//! which is a [`Future`](std::future::Future) and nothing else. A thread cannot
//! wait on one without an executor to poll it, so the blocking backend uses the
//! [`oneshot`] crate instead, whose receiver blocks a thread and can also be
//! polled wait-free. That is one heap allocation for the channel, the same as
//! the futures oneshot costs, so the per-call budget is the same either way.
//!
//! Only its `std` feature is on. `try_recv`, which [`Wait::Spin`] loops on,
//! needs no feature at all; `async` is deliberately off, since a blocking
//! handle's methods are plain `fn` and never await.
//!
//! What is actify's own, and all that is left here, is the choice between
//! parking and busy-waiting.

use std::any::type_name;
use std::fmt::{self, Debug};
use std::hint;

use oneshot::TryRecvError;

use super::Wait;
use super::channel::Closed;

/// The half of a call's reply channel that the actor holds.
///
/// It carries the concrete return type, so a result travels home as itself
/// rather than as a boxed `Any` the caller has to downcast. Answering it is
/// [`Actor::respond`](crate::__private::Actor::respond).
pub struct Reply<R>(oneshot::Sender<R>);

impl<R> Debug for Reply<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Reply<{}>", type_name::<R>())
    }
}

/// Both backends answer a call the same way, which is what lets them share
/// [`Actor::respond`](crate::__private::Actor::respond).
impl<R> crate::actor::Answerable<R> for Reply<R> {
    fn answer(self, value: R) -> Result<(), ()> {
        self.0.send(value).map_err(|_| ())
    }
}

/// The half of a call's reply channel that the caller holds.
///
/// It is consumed by waiting, which is the only thing it is for. Dropping it
/// unwaited tells the actor the work was wasted, which
/// [`Actor::respond`](crate::__private::Actor::respond) logs.
pub struct Answer<R>(oneshot::Receiver<R>);

impl<R> Debug for Answer<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Answer<{}>", type_name::<R>())
    }
}

impl<R> Answer<R> {
    /// Waits for the reply the way the handle was built to wait.
    ///
    /// Reports [`Closed`] when the actor dropped its [`Reply`] without
    /// answering, which is what a stopped actor and a panicking method both
    /// look like from here.
    pub fn recv(self, wait: Wait) -> Result<R, Closed> {
        match wait {
            // Blocks the thread, costing nothing while it waits.
            Wait::Park => self.0.recv().map_err(|_| Closed),
            // Busy-waits, never sleeping. This is the lowest latency a reply
            // can have and the most expensive way to wait: the thread holds
            // its core for the whole call.
            Wait::Spin => loop {
                match self.0.try_recv() {
                    Ok(value) => return Ok(value),
                    Err(TryRecvError::Empty) => hint::spin_loop(),
                    Err(TryRecvError::Disconnected) => return Err(Closed),
                }
            },
        }
    }
}

/// Creates the two halves of one call's reply channel.
///
/// The actor keeps the [`Reply`], the caller waits on the [`Answer`].
#[doc(hidden)]
pub fn reply<R>() -> (Reply<R>, Answer<R>) {
    let (sender, receiver) = oneshot::channel();
    (Reply(sender), Answer(receiver))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor::Answerable;
    use std::thread;
    use std::time::Duration;

    /// Both modes take the answer. The channel is the `oneshot` crate's to
    /// test; what is tested here is that each mode reaches it.
    #[test]
    fn test_either_way_of_waiting_gets_the_answer() {
        for wait in [Wait::Park, Wait::Spin] {
            let (reply, answer) = reply();
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(20));
                let _ = reply.answer(7);
            });
            assert_eq!(answer.recv(wait), Ok(7), "{wait:?}");
        }
    }

    #[test]
    fn test_an_answer_already_sent_is_taken_without_waiting() {
        for wait in [Wait::Park, Wait::Spin] {
            let (reply, answer) = reply();
            let _ = reply.answer(7);
            assert_eq!(answer.recv(wait), Ok(7), "{wait:?}");
        }
    }

    /// The actor stopping, or a method panicking, drops the reply. Both modes
    /// have to report that rather than wait forever.
    #[test]
    fn test_a_dropped_reply_closes_the_call() {
        for wait in [Wait::Park, Wait::Spin] {
            let (reply, answer) = reply::<u8>();
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(20));
                drop(reply);
            });
            assert_eq!(answer.recv(wait), Err(Closed), "{wait:?}");
        }
    }

    #[test]
    fn test_a_reply_dropped_before_the_wait_closes_the_call() {
        for wait in [Wait::Park, Wait::Spin] {
            let (reply, answer) = reply::<u8>();
            drop(reply);
            assert_eq!(answer.recv(wait), Err(Closed), "{wait:?}");
        }
    }

    /// A caller that gave up is worth a line on the actor's side, because it
    /// means the work was wasted.
    #[test]
    fn test_answering_a_caller_that_gave_up_reports_it() {
        let (reply, answer) = reply();
        drop(answer);
        assert_eq!(reply.answer(7), Err(()));
    }
}
