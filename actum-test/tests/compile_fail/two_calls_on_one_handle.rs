// EXPECTED: a handle sends through `&mut`, so one handle cannot have two calls
// in flight at once. Concurrent callers each need a handle, which is what makes
// a bounded channel's ceiling something the program sets.
use actum::actum;

#[derive(Clone, Debug)]
struct Counter(i32);

#[actum]
impl Counter {
    fn add(&mut self, value: i32) -> i32 {
        self.0 += value;
        self.0
    }
}

#[tokio::main]
async fn main() {
    let mut handle = CounterHandle::new(Counter(0));

    let first = handle.add(1);
    let second = handle.add(2);

    let _ = (first.await, second.await);
}
