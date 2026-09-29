// EXPECTED: every generated handle already has `new` and `builder`, so an actor
// method of the same name would define it on the handle twice.
use actum::actum;

#[derive(Debug)]
struct Store {
    items: Vec<i32>,
}

#[actum]
impl Store {
    fn new(&self) -> usize {
        self.items.len()
    }

    fn builder(&self) -> usize {
        self.items.len()
    }
}

fn main() {}
