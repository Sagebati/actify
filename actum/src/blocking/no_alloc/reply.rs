//! A call's reply, on the caller's stack.
//!
//! The slot is a [`heapless`] single-producer, single-consumer queue that
//! holds one value. The actor's half is a producer into it, carried inside the
//! message; the caller's half is the consumer, which never leaves the frame
//! the slot lives in. Nothing here is on the heap.
//!
//! # Why this is sound
//!
//! The message enum is `'static`, so the producer inside it has to be too,
//! and [`Slot::split`] makes it so with a `transmute` of its lifetime. What
//! keeps that honest is the pair of rules below, one per half:
//!
//! - A [`Reply`] touches the slot exactly once, to enqueue either the answer
//!   or [`Closed`], and never after. Answering consumes it; dropping it
//!   unanswered enqueues `Closed`; a `Reply` that has answered does nothing
//!   when dropped.
//! - An [`Answer`] does not release its borrow of the slot until that one
//!   enqueue has been observed, whether it is received through
//!   [`Answer::recv`] or dropped on the way out of an unwinding frame, in
//!   which case its `Drop` waits instead.
//!
//! Between them: for as long as the producer can still write, the consumer is
//! alive, and so is the slot it borrows. A message that is leaked rather than
//! dropped leaves the caller waiting forever, which keeps the slot alive too:
//! a leak is a hang, never a dangling pointer.

use std::any::type_name;
use std::fmt::{self, Debug};
use std::hint;
use std::marker::PhantomData;
use std::mem;
use std::thread::{self, Thread};

use heapless::spsc::{Consumer, Producer, Queue};

use super::super::Wait;
use super::super::channel::Closed;
use crate::actor::Answerable;

/// Where a call's reply lands: a local of the generated forwarder.
///
/// Two slots deep because that is the ring buffer's minimum; it holds one
/// value, which is all a call gets.
pub struct Slot<R>(Queue<Result<R, Closed>, 2>);

impl<R: 'static> Slot<R> {
    /// An empty slot.
    pub fn new() -> Self {
        Slot(Queue::new())
    }

    /// The actor's half and the caller's half.
    ///
    /// The caller's half borrows the slot; the actor's half is made to look
    /// `'static` so it can travel inside the message. The module docs say why
    /// that is sound; in short, the caller's half waits for the actor's half
    /// to be done with the slot before it lets go of it.
    pub fn split(&mut self) -> (Reply<R>, Answer<'_, R>) {
        let (producer, consumer) = self.0.split();
        // SAFETY: The producer's only use is one `enqueue`, from `Reply::deliver`,
        // after which no `Reply` method touches it again. The `Answer` returned
        // beside it borrows `self` for as long as it exists, and both ways it
        // can cease to exist, `recv` and `drop`, first wait until that enqueue
        // has been dequeued. So the slot outlives every access the producer
        // makes, which is what the erased lifetime promised.
        let producer: Producer<'static, Result<R, Closed>> = unsafe { mem::transmute(producer) };
        (
            Reply {
                producer,
                caller: thread::current(),
                answered: false,
            },
            Answer {
                consumer,
                received: false,
                _thread: PhantomData,
            },
        )
    }
}

impl<R: 'static> Default for Slot<R> {
    fn default() -> Self {
        Self::new()
    }
}

impl<R> Debug for Slot<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Slot<{}>", type_name::<R>())
    }
}

/// The actor's half of a reply slot, carried inside the message.
///
/// It carries the concrete return type, so a result travels home as itself.
/// Answering it is [`Actor::respond`](crate::__private::Actor::respond).
pub struct Reply<R: 'static> {
    producer: Producer<'static, Result<R, Closed>>,
    /// The caller, to unpark when it waits by parking. An `Arc` clone: no
    /// allocation.
    caller: Thread,
    answered: bool,
}

impl<R: 'static> Reply<R> {
    /// The one write this half ever makes.
    fn deliver(&mut self, outcome: Result<R, Closed>) {
        debug_assert!(!self.answered, "a reply is delivered once");
        self.answered = true;
        // Cannot be full: the slot holds one value and this is the one write.
        // The `Err` returns the value, which is dropped here in the case that
        // cannot happen.
        let _ = self.producer.enqueue(outcome);
        self.caller.unpark();
    }
}

impl<R: 'static> Answerable<R> for Reply<R> {
    fn answer(mut self, value: R) -> Result<(), ()> {
        self.deliver(Ok(value));
        Ok(())
    }
}

impl<R: 'static> Drop for Reply<R> {
    fn drop(&mut self) {
        if !self.answered {
            self.deliver(Err(Closed));
        }
    }
}

impl<R: 'static> Debug for Reply<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Reply<{}>", type_name::<R>())
    }
}

/// The caller's half of a reply slot.
///
/// It is consumed by waiting, and it waits even when dropped: see the module
/// docs. It stays on the thread that made it, since that is the thread the
/// [`Reply`] unparks, so it is not `Send`.
pub struct Answer<'a, R> {
    consumer: Consumer<'a, Result<R, Closed>>,
    received: bool,
    _thread: PhantomData<*const ()>,
}

impl<R> Answer<'_, R> {
    /// Waits for the reply the way the handle was built to wait.
    ///
    /// Reports [`Closed`] when the actor dropped its [`Reply`] without
    /// answering, which is what a stopped actor and a panicking method both
    /// look like from here.
    pub fn recv(mut self, wait: Wait) -> Result<R, Closed> {
        let outcome = self.wait_for(wait);
        self.received = true;
        outcome
    }

    fn wait_for(&mut self, wait: Wait) -> Result<R, Closed> {
        loop {
            if let Some(outcome) = self.consumer.dequeue() {
                return outcome;
            }
            match wait {
                // A stale unpark token or a spurious wakeup only loops again.
                Wait::Park => thread::park(),
                // The lowest latency a reply can have and the most expensive
                // way to wait: the thread holds its core for the whole call.
                Wait::Spin => hint::spin_loop(),
            }
        }
    }
}

impl<R> Drop for Answer<'_, R> {
    fn drop(&mut self) {
        // Only an unwinding frame gets here, so how it waits does not matter;
        // that it waits is what keeps the slot alive for the actor's write.
        if !self.received {
            let _ = self.wait_for(Wait::Park);
        }
    }
}

impl<R> super::super::Receive<R> for Answer<'_, R> {
    fn recv(self, wait: Wait) -> Result<R, Closed> {
        Answer::recv(self, wait)
    }
}

impl<R> Debug for Answer<'_, R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Answer<{}>", type_name::<R>())
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn both_waits_receive_an_answer() {
        for wait in [Wait::Park, Wait::Spin] {
            let mut slot = Slot::new();
            let (reply, answer) = slot.split();
            let answering = thread::spawn(move || {
                thread::sleep(Duration::from_millis(20));
                reply.answer(41 + 1).unwrap();
            });
            assert_eq!(answer.recv(wait), Ok(42), "{wait:?}");
            answering.join().unwrap();
        }
    }

    #[test]
    fn a_reply_dropped_unanswered_is_closed() {
        let mut slot = Slot::<u8>::new();
        let (reply, answer) = slot.split();
        drop(reply);
        assert_eq!(answer.recv(Wait::Park), Err(Closed));
    }

    #[test]
    fn a_reply_dropped_on_another_thread_wakes_a_parked_caller() {
        let mut slot = Slot::<u8>::new();
        let (reply, answer) = slot.split();
        let dropping = thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            drop(reply);
        });
        assert_eq!(answer.recv(Wait::Park), Err(Closed));
        dropping.join().unwrap();
    }

    /// The rule the `transmute` rests on: an answer that is dropped rather
    /// than received still waits for the actor's write.
    #[test]
    fn an_answer_dropped_unreceived_waits_for_the_reply() {
        let mut slot = Slot::new();
        let (reply, answer) = slot.split();
        let delay = Duration::from_millis(100);
        let answering = thread::spawn(move || {
            thread::sleep(delay);
            reply.answer(1).unwrap();
        });

        let started = Instant::now();
        drop(answer);
        assert!(
            started.elapsed() >= delay,
            "dropping the answer returned before the reply was written"
        );
        answering.join().unwrap();
    }
}
