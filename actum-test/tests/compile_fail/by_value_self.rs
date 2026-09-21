// EXPECTED: a clear message that the receiver must be a reference.
// The actor owns its state for its whole lifetime, so a method consuming
// self cannot be dispatched through a handle.
use actum::actum;

#[derive(Clone, Debug)]
struct MyActor;

#[actum]
impl MyActor {
    fn consume(self) -> i32 {
        42
    }
}

fn main() {}
