// EXPECTED: a method cannot declare generic parameters of its own, because a
// call travels as a variant of an enum that has no such parameter.
use actify::actify;

#[derive(Clone, Debug)]
struct MyActor;

#[actify]
impl MyActor {
    fn apply<F>(&self, value: usize, f: F) -> usize
    where
        F: Fn(usize) -> usize + Send + Sync + 'static,
    {
        f(value)
    }
}

fn main() {}
