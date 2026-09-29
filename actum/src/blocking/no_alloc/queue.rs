//! A job queue in a `static`: a ring buffer, and the bookkeeping that makes it
//! a channel.
//!
//! [`heapless`]'s multi-producer, multi-consumer queue is the ring buffer. It
//! knows nothing about senders going away or a receiver waiting, so those are
//! added here: a count of sending halves, a flag for the receiving half, and
//! the receiver's parked thread so a sender can wake it.

use std::any::type_name;
use std::fmt::{self, Debug};
use std::hint;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering, fence};
use std::thread::{self, Thread};

use heapless::mpmc::Queue as Ring;

use super::super::channel::{Closed, JobReceiver, JobSender, Next};

/// A job queue for a `no_alloc` actor, declared in a `static`.
///
/// `N` is the number of jobs it holds, a power of two from 2 to 128 (the
/// ring buffer's own rule, checked when the program is built). A send that
/// finds it full waits for room.
///
/// [`split`](Self::split) gives out the two halves; see the
/// [module docs](super) for how they behave.
pub struct Queue<M, const N: usize> {
    jobs: Ring<M, N>,
    /// Live sending halves. At zero the receiving half reads `Closed` once
    /// the ring is empty.
    senders: AtomicUsize,
    /// Whether `split` has handed the receiving half out.
    receiver_taken: AtomicBool,
    /// Set when the receiving half is dropped: a send fails from then on.
    receiver_gone: AtomicBool,
    /// The receiver's thread while it is parked, for a sender to wake.
    parked: Mutex<Option<Thread>>,
}

impl<M, const N: usize> Queue<M, N> {
    /// An empty queue. `const`, so it can initialise a `static`.
    pub const fn new() -> Self {
        Queue {
            jobs: Ring::new(),
            senders: AtomicUsize::new(0),
            receiver_taken: AtomicBool::new(false),
            receiver_gone: AtomicBool::new(false),
            parked: Mutex::new(None),
        }
    }

    /// Both halves: the sending one for the handle, the receiving one for the
    /// actor, in the order the builder's `channel` takes them.
    ///
    /// # Panics
    ///
    /// If called twice. There is one receiving half; more sending halves come
    /// from cloning the first.
    pub fn split(&'static self) -> (Sender<M, N>, Receiver<M, N>) {
        assert!(
            !self.receiver_taken.swap(true, Ordering::AcqRel),
            "a no_alloc Queue is split once: it has one receiving half"
        );
        self.senders.fetch_add(1, Ordering::SeqCst);
        (Sender { queue: self }, Receiver { queue: self })
    }

    /// How many jobs the queue holds before a send waits.
    pub const fn capacity(&self) -> usize {
        N
    }

    /// Wakes the receiver if it is parked.
    ///
    /// The lock is what orders this against the receiver's own steps: a
    /// receiver that registers itself under the lock and then looks at the
    /// queue sees whatever a sender did before taking the lock, and a sender
    /// that finds nobody registered knows the receiver has not yet parked.
    fn wake_receiver(&self) {
        if let Some(receiver) = self.parked.lock().unwrap().as_ref() {
            receiver.unpark();
        }
    }

    /// Drops every job in the ring, which tells each job's caller the actor is
    /// gone.
    fn drain(&self) {
        while self.jobs.dequeue().is_some() {}
    }
}

impl<M, const N: usize> Default for Queue<M, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<M, const N: usize> Debug for Queue<M, N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Queue")
            .field("message", &type_name::<M>())
            .field("capacity", &N)
            .field("senders", &self.senders.load(Ordering::Relaxed))
            .finish()
    }
}

/// The sending half of a [`Queue`], which a handle holds.
///
/// Cloning it is what cloning the handle does; the queue counts them, and the
/// actor stops once the last one is gone.
pub struct Sender<M: 'static, const N: usize> {
    queue: &'static Queue<M, N>,
}

impl<M: 'static, const N: usize> Clone for Sender<M, N> {
    fn clone(&self) -> Self {
        self.queue.senders.fetch_add(1, Ordering::SeqCst);
        Sender { queue: self.queue }
    }
}

impl<M: 'static, const N: usize> Drop for Sender<M, N> {
    fn drop(&mut self) {
        // The last sender wakes the receiver so it can read `Closed`.
        if self.queue.senders.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.queue.wake_receiver();
        }
    }
}

impl<M: 'static, const N: usize> Debug for Sender<M, N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Sender<{}, {N}>", type_name::<M>())
    }
}

/// How many spins a sender takes on a full queue before yielding its core.
const SPINS_BEFORE_YIELD: u32 = 64;

impl<M: Send + 'static, const N: usize> JobSender<M> for Sender<M, N> {
    fn send(&self, job: M) -> Result<(), Closed> {
        let queue = self.queue;
        if queue.receiver_gone.load(Ordering::SeqCst) {
            return Err(Closed);
        }

        let mut job = job;
        let mut spins = 0;
        while let Err(returned) = queue.jobs.enqueue(job) {
            // Full. The receiver will make room, unless it is gone.
            job = returned;
            if queue.receiver_gone.load(Ordering::SeqCst) {
                return Err(Closed);
            }
            if spins < SPINS_BEFORE_YIELD {
                spins += 1;
                hint::spin_loop();
            } else {
                thread::yield_now();
            }
        }

        // The receiver may have gone between the check above and the
        // enqueue. This fence and the one in `Receiver::drop` order the two:
        // either this load sees the flag, or the receiver's drain sees the
        // job. Both can be true; neither can be false.
        fence(Ordering::SeqCst);
        if queue.receiver_gone.load(Ordering::SeqCst) {
            queue.drain();
            return Err(Closed);
        }

        queue.wake_receiver();
        Ok(())
    }
}

/// The receiving half of a [`Queue`], which the actor reads.
///
/// There is one. Dropping it drops every job still queued, which tells each
/// of their callers the actor is gone, and fails every send from then on.
pub struct Receiver<M: 'static, const N: usize> {
    queue: &'static Queue<M, N>,
}

impl<M: 'static, const N: usize> Drop for Receiver<M, N> {
    fn drop(&mut self) {
        self.queue.receiver_gone.store(true, Ordering::SeqCst);
        // Pairs with the fence in `Sender::send`; see there.
        fence(Ordering::SeqCst);
        self.queue.drain();
    }
}

impl<M: 'static, const N: usize> Debug for Receiver<M, N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Receiver<{}, {N}>", type_name::<M>())
    }
}

impl<M: Send + 'static, const N: usize> JobReceiver<M> for Receiver<M, N> {
    fn recv(&mut self) -> Option<M> {
        loop {
            match self.try_recv() {
                Next::Job(job) => return Some(job),
                Next::Closed => return None,
                Next::Empty => {}
            }

            // Register, then look once more: a sender that enqueued before
            // seeing the registration is seen here, and one that enqueues
            // after it will unpark. Either way the park below cannot miss.
            *self.queue.parked.lock().unwrap() = Some(thread::current());
            let found = self.try_recv();
            if matches!(found, Next::Empty) {
                thread::park();
            }
            *self.queue.parked.lock().unwrap() = None;

            match found {
                Next::Job(job) => return Some(job),
                Next::Closed => return None,
                Next::Empty => {}
            }
        }
    }

    fn try_recv(&mut self) -> Next<M> {
        let queue = self.queue;
        if let Some(job) = queue.jobs.dequeue() {
            return Next::Job(job);
        }
        if queue.senders.load(Ordering::SeqCst) == 0 {
            // The last sender may have enqueued and then dropped between the
            // two reads above, so the ring gets one more look.
            fence(Ordering::SeqCst);
            return match queue.jobs.dequeue() {
                Some(job) => Next::Job(job),
                None => Next::Closed,
            };
        }
        Next::Empty
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

    /// A job that tells a channel when it is dropped, standing in for a reply
    /// half that reports `Closed` to its caller.
    struct Job(mpsc::Sender<&'static str>);

    impl Drop for Job {
        fn drop(&mut self) {
            let _ = self.0.send("dropped");
        }
    }

    #[test]
    fn jobs_arrive_in_order() {
        static QUEUE: Queue<u32, 4> = Queue::new();
        let (tx, mut rx) = QUEUE.split();

        for i in 0..4 {
            tx.send(i).unwrap();
        }
        assert_eq!(rx.try_recv(), Next::Job(0));
        for i in 1..4 {
            assert_eq!(rx.recv(), Some(i));
        }
        assert_eq!(rx.try_recv(), Next::Empty);
    }

    #[test]
    fn closed_once_every_sender_is_gone() {
        static QUEUE: Queue<u32, 2> = Queue::new();
        let (tx, mut rx) = QUEUE.split();
        let other = tx.clone();

        tx.send(1).unwrap();
        drop(tx);
        assert_eq!(rx.try_recv(), Next::Job(1));
        assert_eq!(rx.try_recv(), Next::Empty, "a clone is still alive");

        drop(other);
        assert_eq!(rx.try_recv(), Next::Closed);
        assert_eq!(rx.recv(), None);
    }

    #[test]
    fn a_parked_receiver_is_woken_by_a_send_and_by_the_last_drop() {
        static QUEUE: Queue<u32, 2> = Queue::new();
        let (tx, mut rx) = QUEUE.split();

        let receiving = thread::spawn(move || {
            let first = rx.recv();
            let second = rx.recv();
            (first, second)
        });
        thread::sleep(Duration::from_millis(50));
        tx.send(7).unwrap();
        thread::sleep(Duration::from_millis(50));
        drop(tx);

        assert_eq!(receiving.join().unwrap(), (Some(7), None));
    }

    #[test]
    fn a_dropped_receiver_drops_queued_jobs_and_fails_later_sends() {
        static QUEUE: Queue<Job, 2> = Queue::new();
        let (tx, rx) = QUEUE.split();
        let (report, reports) = mpsc::channel();

        tx.send(Job(report.clone())).unwrap();
        drop(rx);
        assert_eq!(
            reports.try_recv(),
            Ok("dropped"),
            "the queued job was dropped"
        );

        assert_eq!(tx.send(Job(report)).unwrap_err(), Closed);
        assert_eq!(
            reports.try_recv(),
            Ok("dropped"),
            "the refused job was dropped"
        );
    }

    #[test]
    fn a_full_queue_waits_for_room() {
        static QUEUE: Queue<u32, 2> = Queue::new();
        let (tx, mut rx) = QUEUE.split();

        tx.send(1).unwrap();
        tx.send(2).unwrap();

        let sending = thread::spawn(move || {
            tx.send(3).unwrap();
        });
        thread::sleep(Duration::from_millis(50));
        assert!(!sending.is_finished(), "the third send waits");

        assert_eq!(rx.recv(), Some(1));
        sending.join().unwrap();
        assert_eq!(rx.recv(), Some(2));
        assert_eq!(rx.recv(), Some(3));
    }

    #[test]
    #[should_panic(expected = "split once")]
    fn a_queue_is_split_once() {
        static QUEUE: Queue<u32, 2> = Queue::new();
        let _first = QUEUE.split();
        let _second = QUEUE.split();
    }
}
