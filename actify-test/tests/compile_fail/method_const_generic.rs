// EXPECTED: a const parameter is a method generic like any other, and the
// field type of a variant cannot depend on it.
use actify::actify;

#[derive(Clone, Debug)]
struct MyActor;

#[actify]
impl MyActor {
    fn total<const N: usize>(&self, values: [u8; N]) -> usize {
        values.iter().map(|b| *b as usize).sum()
    }
}

fn main() {}
