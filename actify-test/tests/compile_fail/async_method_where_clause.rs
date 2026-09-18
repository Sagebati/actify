// EXPECTED: a where clause is carried by a function pointer to the method, and
// one to an async method returns a future the message cannot hold.
use actify::actify;

#[derive(Clone, Debug)]
struct MyActor;

#[actify]
impl MyActor {
    async fn sorted(&self) -> Vec<i32>
    where
        i32: Ord,
    {
        vec![1, 2, 3]
    }
}

fn main() {}
