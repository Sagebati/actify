//! A call's reply channel, for a caller that has no executor.
//!
//! The async backend hands the caller a [`futures_channel::oneshot::Receiver`],
//! which is a [`Future`](std::future::Future) and nothing else. A thread cannot
//! wait on one without an executor to poll it, so the blocking backend brings
//! its own one-shot slot: a single [`Arc`] holding the value, a mutex, a
//! condvar for the parking caller and an atomic flag for the spinning one.
//!
//! That is the same one allocation a futures oneshot costs, so the blocking
//! backend's per-call budget matches the async one.

use std::any::type_name;
use std::fmt::{self, Debug};
use std::hint;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use super::Wait;
use crate::channel::Closed;

/// What the slot holds, once anything does.
enum State<R> {
    /// Nobody has answered yet.
    Empty,
    /// The actor answered.
    Filled(R),
    /// The actor will never answer: it stopped, or its method panicked.
    Closed,
}

/// The one allocation a call costs.
struct Slot<R> {
    /// Set last by the writer and read first by a spinning caller, so a spin
    /// never touches the mutex until there is something to take.
    done: AtomicBool,
    state: Mutex<State<R>>,
    ready: Condvar,
}

impl<R> Slot<R> {
    /// The state, whether or not a previous holder of the lock panicked.
    ///
    /// Poisoning carries no information here: the slot holds one value that is
    /// written once, and a panicking actor method drops its [`Reply`] rather
    /// than leaving a half-written one behind.
    fn lock(&self) -> std::sync::MutexGuard<'_, State<R>> {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// Publishes a final state and wakes a parked caller.
    fn finish(&self, state: State<R>) {
        let mut guard = self.lock();
        *guard = state;
        self.done.store(true, Ordering::Release);
        self.ready.notify_one();
    }
}

/// The half of a call's reply channel that the actor holds.
///
/// It carries the concrete return type, so a result travels home as itself
/// rather than as a boxed `Any` the caller has to downcast. Answering it is
/// [`Actor::respond`](crate::__private::Actor::respond).
pub struct Reply<R>(Arc<Slot<R>>);

impl<R> Debug for Reply<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Reply<{}>", type_name::<R>())
    }
}

impl<R> Reply<R> {
    /// Hands the value over and wakes the caller, reporting whether anyone was
    /// still there to take it.
    ///
    /// Taking `self` is what lets [`Drop`] tell an answered call from an
    /// abandoned one without a flag of its own.
    pub(crate) fn send(self, value: R) -> Result<(), ()> {
        let mut guard = self.0.lock();
        if matches!(*guard, State::Closed) {
            // The caller gave up first, which means the work was wasted.
            return Err(());
        }
        *guard = State::Filled(value);
        // Published under the lock, and notified under it too: a caller can
        // only be waiting if it took this lock first, and it re-checks the
        // state under it, so a wakeup cannot go missing.
        self.0.done.store(true, Ordering::Release);
        self.0.ready.notify_one();
        Ok(())
    }
}

impl<R> Drop for Reply<R> {
    fn drop(&mut self) {
        // Already answered, so `send` ran and the state is `Filled`.
        if self.0.done.load(Ordering::Acquire) {
            return;
        }
        // Not answered, so the actor is gone: its loop ended between taking
        // the call and answering it, or the method panicked and this is
        // unwinding out of it. Either way the caller has to stop waiting.
        self.0.finish(State::Closed);
    }
}

/// The half of a call's reply channel that the caller holds.
///
/// It is consumed by waiting, which is the only thing it is for.
pub struct Answer<R>(Arc<Slot<R>>);

impl<R> Debug for Answer<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Answer<{}>", type_name::<R>())
    }
}

impl<R> Answer<R> {
    /// Waits for the reply the way the handle was built to wait.
    pub(crate) fn recv(self, wait: Wait) -> Result<R, Closed> {
        match wait {
            Wait::Park => self.park(),
            Wait::Spin => self.spin(),
        }
    }

    /// Blocks the thread until the actor answers. Costs nothing while waiting.
    fn park(&self) -> Result<R, Closed> {
        let mut guard = self.0.lock();
        loop {
            match std::mem::replace(&mut *guard, State::Empty) {
                State::Empty => {
                    guard = self
                        .0
                        .ready
                        .wait(guard)
                        .unwrap_or_else(|poison| poison.into_inner())
                }
                State::Filled(value) => return Ok(value),
                State::Closed => {
                    *guard = State::Closed;
                    return Err(Closed);
                }
            }
        }
    }

    /// Busy-waits until the actor answers, without ever sleeping.
    ///
    /// This is the lowest latency a reply can have and the most expensive way
    /// to wait: the thread holds its core for the whole call. It is for a
    /// pinned thread that has nothing else to do.
    fn spin(&self) -> Result<R, Closed> {
        while !self.0.done.load(Ordering::Acquire) {
            hint::spin_loop();
        }
        let mut guard = self.0.lock();
        match std::mem::replace(&mut *guard, State::Closed) {
            State::Filled(value) => Ok(value),
            // `done` is only set once the state is final, and neither final
            // state is `Empty`.
            State::Empty | State::Closed => Err(Closed),
        }
    }
}

impl<R> Drop for Answer<R> {
    fn drop(&mut self) {
        // Answered already, so there is nothing to give up on: either the
        // caller took the value, or it is still sitting in the slot and goes
        // away with it.
        if self.0.done.load(Ordering::Acquire) {
            return;
        }
        // The caller gave up before the actor answered. Recording it here is
        // what lets `Reply::send` report that the work was wasted. No notify:
        // the only thread that reads this is the one about to answer, and it
        // takes the lock to do so.
        *self.0.lock() = State::Closed;
    }
}

/// Both backends answer a call the same way, which is what lets them share
/// [`Actor::respond`](crate::__private::Actor::respond).
impl<R> crate::actor::Answerable<R> for Reply<R> {
    fn answer(self, value: R) -> Result<(), ()> {
        self.send(value)
    }
}

/// Creates the two halves of one call's reply channel.
///
/// The actor keeps the [`Reply`], the caller waits on the [`Answer`].
#[doc(hidden)]
pub fn reply<R>() -> (Reply<R>, Answer<R>) {
    let slot = Arc::new(Slot {
        done: AtomicBool::new(false),
        state: Mutex::new(State::Empty),
        ready: Condvar::new(),
    });
    (Reply(Arc::clone(&slot)), Answer(slot))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_a_parked_caller_gets_the_answer() {
        let (reply, answer) = reply();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            let _ = reply.send(7);
        });
        assert_eq!(answer.recv(Wait::Park), Ok(7));
    }

    #[test]
    fn test_a_spinning_caller_gets_the_answer() {
        let (reply, answer) = reply();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            let _ = reply.send(7);
        });
        assert_eq!(answer.recv(Wait::Spin), Ok(7));
    }

    #[test]
    fn test_an_answer_already_sent_is_taken_without_waiting() {
        let (reply, answer) = reply();
        let _ = reply.send(7);
        assert_eq!(answer.recv(Wait::Park), Ok(7));
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
        let (reply, answer) = reply::<u8>();
        drop(reply);
        assert_eq!(answer.recv(Wait::Spin), Err(Closed));
    }
}
