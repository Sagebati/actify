//! A pool of actors on one queue: many producers, many consumers.
//!
//! An actor owns its receiving half, so one actor is one consumer. A pool is
//! several actors whose receiving halves are clones of one channel: a job sent
//! through any handle lands on whichever worker takes it next, and the reply
//! finds its way back to the caller whoever did the work.
//!
//! That needs a channel whose receiver clones, which neither default channel's
//! does. This brings `flume`, whose sender sends through `&self` and whose
//! receiver clones, and implements the two blocking channel traits for it -
//! the bring-your-own-channel the traits are public for, in about a dozen
//! lines. The traits and `flume`'s types are both foreign to this file, so
//! the impls go on two newtypes.
//!
//! Run it with `cargo run --example worker_pool`.

use actum::actum;
use actum::blocking::{Closed, JobReceiver, JobSender, Next};

const WORKERS: usize = 4;
const PRODUCERS: usize = 10;
const JOBS_EACH: usize = 25;

/// The pool's queue, as a handle holds it.
struct Queue<M>(flume::Sender<M>);

impl<M> Clone for Queue<M> {
    fn clone(&self) -> Self {
        Queue(self.0.clone())
    }
}

impl<M: Send + 'static> JobSender<M> for Queue<M> {
    fn send(&self, job: M) -> Result<(), Closed> {
        self.0.send(job).map_err(|_| Closed)
    }
}

/// The pool's queue, as each worker reads it. Every clone reads the same
/// queue, which is what makes it a pool.
struct Jobs<M>(flume::Receiver<M>);

impl<M> Clone for Jobs<M> {
    fn clone(&self) -> Self {
        Jobs(self.0.clone())
    }
}

impl<M: Send + 'static> JobReceiver<M> for Jobs<M> {
    fn recv(&mut self) -> Option<M> {
        self.0.recv().ok()
    }

    fn try_recv(&mut self) -> Next<M> {
        match self.0.try_recv() {
            Ok(job) => Next::Job(job),
            Err(flume::TryRecvError::Empty) => Next::Empty,
            Err(flume::TryRecvError::Disconnected) => Next::Closed,
        }
    }
}

/// One worker. Each has state of its own - here, its id and a tally - and
/// none of them share anything but the queue.
struct Worker {
    id: usize,
    done: usize,
}

#[actum(blocking)]
impl Worker {
    /// Does one job and says who did it.
    fn run(&mut self, job: u64) -> (usize, u64) {
        self.done += 1;
        // Enough work that no single worker can take the whole queue before
        // the others wake, so the spread below is a real one.
        let digest = (0..2_000u64).fold(job, |h, i| h.wrapping_mul(31).wrapping_add(i));
        (self.id, digest)
    }
}

fn main() {
    let (tx, rx) = flume::unbounded();

    // One actor per worker, each reading a clone of the same queue. `build`
    // takes one (sender, receiver) pair and returns one handle, so the pool
    // is built in a loop. The handles are interchangeable. `Wait::Park` is the
    // default; `Wait::Spin` would have four idle workers each holding a core.
    let mut handles = Vec::with_capacity(WORKERS);
    let mut threads = Vec::with_capacity(WORKERS);
    for id in 0..WORKERS {
        let (handle, actor) = WorkerHandle::builder(Worker { id, done: 0 })
            .channel((Queue(tx.clone()), Jobs(rx.clone())))
            .build();
        handles.push(handle);
        threads.push(std::thread::spawn(actor));
    }
    // The originals go now, so that the handles are the only senders left and
    // dropping them is what closes the queue for every worker.
    drop((tx, rx));

    // Ten producers, each on a thread with a handle of its own. Which of the
    // four it clones does not matter: every one feeds the same queue.
    let producers: Vec<_> = (0..PRODUCERS)
        .map(|producer| {
            let handle = handles[producer % WORKERS].clone();
            std::thread::spawn(move || {
                let mut served_by = [0usize; WORKERS];
                for j in 0..JOBS_EACH {
                    let (worker, _digest) = handle.run((producer * JOBS_EACH + j) as u64);
                    served_by[worker] += 1;
                }
                served_by
            })
        })
        .collect();

    let mut spread = [0usize; WORKERS];
    for producer in producers {
        for (worker, jobs) in producer.join().unwrap().into_iter().enumerate() {
            spread[worker] += jobs;
        }
    }
    for (worker, jobs) in spread.iter().enumerate() {
        println!("worker {worker}: {jobs} jobs");
    }
    let total: usize = spread.iter().sum();
    println!("total: {total} of {}", PRODUCERS * JOBS_EACH);
    assert_eq!(total, PRODUCERS * JOBS_EACH);

    // The last handle to drop closes the queue, and every worker's loop ends
    // on the same `None`.
    drop(handles);
    for worker in threads {
        worker.join().unwrap();
    }
}
