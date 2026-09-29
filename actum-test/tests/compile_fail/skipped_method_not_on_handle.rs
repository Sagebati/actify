// EXPECTED: a skipped method is absent from the generated handle, so calling
// it through a handle does not compile.
use actum::actum;

#[derive(Clone, Debug)]
struct MyActor {
    value: i32,
}

#[actum]
impl MyActor {
    fn exposed(&self) -> i32 {
        self.value
    }

    #[actum::skip]
    fn hidden(&self) -> i32 {
        self.value
    }
}

#[tokio::main]
async fn main() {
    let handle = MyActorHandle::new(MyActor { value: 1 });

    let _ = handle.exposed().await;
    let _ = handle.hidden().await;
}
