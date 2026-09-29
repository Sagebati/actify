//! A `no_alloc` actor has no default channel, because the default would
//! allocate: `build` exists only once `channel` has been given a queue.

use actum::actum;

struct Counter(i32);

#[actum(blocking, no_alloc)]
impl Counter {
    fn add(&mut self, value: i32) -> i32 {
        self.0 += value;
        self.0
    }
}

fn main() {
    let (_handle, _actor) = CounterHandle::builder(Counter(0)).build();
}
