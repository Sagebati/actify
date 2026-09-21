//! A blocking actor is served through whatever channel the caller supplies, so
//! these run the same actor over each of them.
//!
//! What every case has to show is the same three things: calls arrive, the
//! caller put the actor on a thread itself, and the actor stops once the last
//! handle is dropped.

use std::sync::mpsc::{Receiver, Sender, SyncSender, TryRecvError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use actum::actum;
use actum::blocking::{JobReceiver, JobSender, Next, Wait};

#[derive(Clone, Debug)]
struct Greeter {
    greeting: String,
}

#[actum(blocking)]
impl Greeter {
    fn say_hi(&self, name: String) -> String {
        format!("{} {name}", self.greeting)
    }

    fn shout(&mut self) {
        self.greeting = self.greeting.to_uppercase();
    }
}

fn greeter() -> Greeter {
    Greeter {
        greeting: "hi".to_string(),
    }
}

/// Waits for the actor's thread to finish, which it does only once the last
/// handle has been dropped. Fails rather than hanging if it never does.
fn stops(running: JoinHandle<()>) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !running.is_finished() {
        assert!(
            Instant::now() < deadline,
            "the actor stops once its last handle is dropped"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    running.join().unwrap();
}

#[test]
fn test_the_default_channel_serves_an_actor() {
    let (handle, actor) = GreeterHandle::builder(greeter()).build();
    let running = std::thread::spawn(actor);

    assert_eq!(handle.say_hi("Alfred".to_string()), "hi Alfred");
    handle.shout();
    assert_eq!(handle.say_hi("Alfred".to_string()), "HI Alfred");

    drop(handle);
    stops(running);
}

/// A bounded channel is how backpressure is asked for. One slot means the
/// queue is full for most of this, which must slow a caller down rather than
/// fail it.
#[test]
fn test_a_bounded_channel_serves_an_actor() {
    let (handle, actor) = GreeterHandle::builder(greeter())
        .channel(std::sync::mpsc::sync_channel(1))
        .build();
    let running = std::thread::spawn(actor);

    for _ in 0..100 {
        assert_eq!(handle.say_hi("Alfred".to_string()), "hi Alfred");
    }

    drop(handle);
    stops(running);
}

#[test]
fn test_a_busy_waiting_actor_is_served() {
    let (handle, actor) = GreeterHandle::builder(greeter()).wait(Wait::Spin).build();
    let running = std::thread::spawn(actor);

    assert_eq!(handle.say_hi("Alfred".to_string()), "hi Alfred");
    handle.shout();
    assert_eq!(handle.say_hi("Alfred".to_string()), "HI Alfred");

    drop(handle);
    stops(running);
}

/// A channel neither actum nor std wrote. The two traits are public so that
/// a crossbeam, flume or ring-buffer queue is a few lines, and this is those
/// lines: nothing here reaches inside the crate.
struct CountingTx<M>(Sender<M>, &'static std::sync::atomic::AtomicUsize);

impl<M> Clone for CountingTx<M> {
    fn clone(&self) -> Self {
        CountingTx(self.0.clone(), self.1)
    }
}

impl<M: Send + 'static> JobSender<M> for CountingTx<M> {
    fn send(&self, job: M) -> Result<(), actum::blocking::Closed> {
        self.1.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.0.send(job).map_err(|_| actum::blocking::Closed)
    }
}

struct CountingRx<M>(Receiver<M>);

impl<M: Send + 'static> JobReceiver<M> for CountingRx<M> {
    fn recv(&mut self) -> Option<M> {
        self.0.recv().ok()
    }

    fn try_recv(&mut self) -> Next<M> {
        match self.0.try_recv() {
            Ok(job) => Next::Job(job),
            Err(TryRecvError::Empty) => Next::Empty,
            Err(TryRecvError::Disconnected) => Next::Closed,
        }
    }
}

static SENT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[test]
fn test_a_channel_written_outside_the_crate_serves_an_actor() {
    let (tx, rx) = std::sync::mpsc::channel();
    let (handle, actor) = GreeterHandle::builder(greeter())
        .channel((CountingTx(tx, &SENT), CountingRx(rx)))
        .build();
    let running = std::thread::spawn(actor);

    assert_eq!(handle.say_hi("Alfred".to_string()), "hi Alfred");
    handle.shout();

    drop(handle);
    stops(running);

    assert_eq!(SENT.load(std::sync::atomic::Ordering::Relaxed), 2);
}

/// `SyncSender` is the other half of the bounded case, named on its own so a
/// change to either impl is caught.
#[test]
fn test_both_std_senders_are_job_senders() {
    fn assert_sender<M, S: JobSender<M>>() {}
    assert_sender::<GreeterCall, Sender<GreeterCall>>();
    assert_sender::<GreeterCall, SyncSender<GreeterCall>>();
}
