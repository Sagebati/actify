// Unsupported argument types (e.g. slices) should be rejected
// with a clear error message suggesting concrete owned types.
use actum::actum;

#[derive(Clone, Debug)]
struct SliceActor;

#[actum]
impl SliceActor {
    fn with_slice(&self, data: [u8]) -> u8 {
        data[0]
    }
}

fn main() {}
