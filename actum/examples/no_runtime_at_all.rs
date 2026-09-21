//! An actor with no executor anywhere: no runtime, no futures, no `.await`.
//!
//! `#[actum(blocking)]` puts the actor on a `std::thread` and makes the
//! handle's methods ordinary calls. This is the whole of what a caller needs to
//! know, in a plain `fn main`.
//!
//! Run it with `cargo run --example no_runtime_at_all`.

use actum::actum;
use actum::blocking::Wait;

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

fn main() {
    // A bounded channel, so callers wait rather than queue without limit. The
    // default is unbounded; either half can come from any blocking channel,
    // since `JobSender` and `JobReceiver` are public.
    let channel = std::sync::mpsc::sync_channel(32);

    let (handle, actor) = GreeterHandle::builder(Greeter {
        greeting: "hi".to_string(),
    })
    .channel(channel)
    // Park, so the actor's thread costs nothing while it is idle. `Wait::Spin`
    // trades that idle cost for the lowest reply latency there is.
    .wait(Wait::Park)
    .build();

    // Nothing runs until the closure does, and the thread is the caller's.
    let served = std::thread::spawn(actor);

    println!("{}", handle.say_hi("Alfred".to_string()));

    handle.shout();
    println!("{}", handle.say_hi("Alfred".to_string()));

    // A second thread sharing the same actor, which is the point of a handle.
    let other = handle.clone();
    let elsewhere = std::thread::spawn(move || other.say_hi("Bertha".to_string()));
    println!("{}", elsewhere.join().unwrap());

    // The actor stops once the last handle is gone, which ends its loop and
    // with it the thread serving it.
    drop(handle);
    served.join().unwrap();
}
