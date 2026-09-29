// EXPECTED: a clear message that impl Trait is not supported as a return type.
// The generated code names the return type in a let binding, where an
// opaque type is not valid syntax.
use actum::actum;

#[derive(Clone, Debug)]
struct MyActor;

#[actum]
impl MyActor {
    fn counter(&self) -> impl Iterator<Item = u8> {
        core::iter::once(1)
    }
}

fn main() {}
