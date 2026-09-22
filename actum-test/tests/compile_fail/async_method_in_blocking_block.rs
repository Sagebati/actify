// EXPECTED: a blocking actor runs on a thread with no executor, so there is
// nothing to drive the future an async method returns.
use actum::actum;

#[derive(Clone, Debug)]
struct MyActor;

#[actum(blocking)]
impl MyActor {
    async fn fetch(&self) -> i32 {
        1
    }
}

fn main() {}
