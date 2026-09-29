// EXPECTED: invalid `name` value: must be a valid Rust identifier
use actum::actum;

#[derive(Clone, Debug)]
struct MyActor;

#[actum(name = "123invalid")]
impl MyActor {
    fn some_method(&self) {}
}

fn main() {}
