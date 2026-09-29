// EXPECTED: every invalid argument is reported, not just the first one.
// Reporting one error per compile cycle forces users to fix a block of
// methods one recompile at a time.
use actum::actum;

#[derive(Clone, Debug)]
struct MyActor;

#[actum]
impl MyActor {
    fn first(&self, _bad_ref: &str) {}

    fn second(&self, _bad_ptr: *const u8) {}

    fn third(&self, _bad_impl: impl Fn()) {}
}

fn main() {}
