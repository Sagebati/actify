// EXPECTED: a blocking actor runs on a thread with no executor, so there is
// nothing to drive the future an async method returns.
use actify::actify;

#[derive(Clone, Debug)]
struct MyActor;

#[actify(blocking)]
impl MyActor {
    async fn fetch(&self) -> i32 {
        1
    }
}

fn main() {}
