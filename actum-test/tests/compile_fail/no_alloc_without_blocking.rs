//! `no_alloc` is a property of the blocking backend: an async call is a future
//! that can be dropped mid-flight, so its reply cannot live on a stack.

use actum::actum;

struct Counter(i32);

#[actum(no_alloc)]
impl Counter {
    fn add(&mut self, value: i32) -> i32 {
        self.0 += value;
        self.0
    }
}

fn main() {}
