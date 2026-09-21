// EXPECTED: every generated handle already has `get`, `set` and `read_handle`,
// so an actor method of the same name would define it on the handle twice.
use actify::actify;

#[derive(Clone, Debug)]
struct Store {
    items: Vec<i32>,
}

#[actify]
impl Store {
    fn get(&self, index: usize) -> Option<i32> {
        self.items.get(index).copied()
    }

    fn builder(&self) -> usize {
        self.items.len()
    }
}

fn main() {}
