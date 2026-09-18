//! An actor is served through whatever channel the caller supplies, so these
//! run the same actor over each channel crate's queue.
//!
//! What every case has to show is the same three things: calls arrive, the
//! caller spawned the actor itself, and the actor stops once the last handle
//! is dropped.

use actify::actify;

#[derive(Clone, Debug)]
struct Greeter {
    greeting: String,
}

#[actify]
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

/// Waits for the actor future to finish, which it does only once the last
/// handle has been dropped. Fails rather than hanging if it never does.
async fn stops(actor: impl Future<Output = ()>) {
    tokio::time::timeout(std::time::Duration::from_secs(5), actor)
        .await
        .expect("the actor stops once its last handle is dropped");
}

#[tokio::test]
async fn test_the_default_channel_serves_an_actor() {
    let (handle, actor) = GreeterHandle::builder(greeter()).build();
    let task = tokio::spawn(actor);

    assert_eq!(handle.say_hi("Alfred".to_string()).await, "hi Alfred");
    handle.shout().await;
    assert_eq!(handle.say_hi("Alfred".to_string()).await, "HI Alfred");

    drop(handle);
    stops(async { task.await.unwrap() }).await;
}

#[tokio::test]
async fn test_flume_serves_an_actor() {
    let (tx, rx) = flume::unbounded();
    let (handle, actor) = GreeterHandle::builder(greeter())
        .channel((tx.into_sink(), rx.into_stream()))
        .build();
    let task = tokio::spawn(actor);

    assert_eq!(handle.say_hi("Alfred".to_string()).await, "hi Alfred");
    handle.shout().await;
    assert_eq!(handle.say_hi("Alfred".to_string()).await, "HI Alfred");

    drop(handle);
    stops(async { task.await.unwrap() }).await;
}

#[tokio::test]
async fn test_a_tokio_mpsc_serves_an_actor() {
    let (tx, rx) = tokio::sync::mpsc::channel(8);
    let (handle, actor) = GreeterHandle::builder(greeter())
        .channel((
            tokio_util::sync::PollSender::new(tx),
            tokio_stream::wrappers::ReceiverStream::new(rx),
        ))
        .build();
    let task = tokio::spawn(actor);

    assert_eq!(handle.say_hi("Alfred".to_string()).await, "hi Alfred");

    drop(handle);
    stops(async { task.await.unwrap() }).await;
}

/// The actor future is inert until it is polled, so an actor can be built on
/// one runtime and served on another. Tokio is only ever the caller's choice.
#[test]
fn test_an_actor_is_served_wherever_the_caller_spawns_it() {
    let (handle, actor) = GreeterHandle::builder(greeter()).build();

    let served = std::thread::spawn(move || futures_executor::block_on(actor));

    let answer = futures_executor::block_on(handle.say_hi("Alfred".to_string()));
    assert_eq!(answer, "hi Alfred");

    drop(handle);
    served.join().unwrap();
}
