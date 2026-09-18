//! Tests actify as any user that imports the library would.

use actify::{ToView, actify};
use std::{collections::HashMap, fmt::Debug, sync::Mutex};

fn main() {}

/// An example struct for the macro tests
#[allow(dead_code)]
#[derive(Clone, Debug)]
struct TestStruct<T> {
    inner_data: T,
}

#[actify]
impl<T> TestStruct<T>
where
    T: Clone + Debug + Send + Sync + 'static,
{
    fn foo(&mut self, i: i32, _h: HashMap<String, T>) -> f64 {
        (i + 1) as f64
    }

    async fn baz(&mut self, i: i32) -> f64 {
        (i + 2) as f64
    }

    fn mut_test(&self, mut arg: String) {
        println!("{arg}");
        arg = "mutated".to_string();
        println!("{arg}")
    }
}

/// An example struct for the macro tests
#[allow(dead_code)]
#[derive(Clone, Debug)]
struct SomeStruct {
    inner_bool: bool,
}

#[actify]
impl SomeStruct {
    fn set_true(&mut self) {
        self.inner_bool = true
    }

    fn set_false(&mut self) {
        self.inner_bool = false
    }

    fn get_inner(&self) -> bool {
        self.inner_bool
    }
}

/// An actor with no methods of its own: an empty `#[actify]` block gives it a
/// handle carrying the built-in calls and nothing else.
#[derive(Clone, Debug, PartialEq)]
struct Plain(i32);

#[actify]
impl Plain {}

#[allow(dead_code)]
/// Example Extension trait
trait TestExt<T> {
    fn extended_foo(&mut self, i: i32, _h: HashMap<String, T>) -> f64;

    fn extended_bar<F>(&mut self, i: usize, f: F) -> usize
    where
        F: Fn(usize) -> usize + Send + Sync + 'static;
}

impl<T> TestExt<T> for TestStruct<T>
where
    T: Clone + Debug + Send + Sync + 'static,
{
    fn extended_foo(&mut self, i: i32, _h: HashMap<String, T>) -> f64 {
        (i + 1) as f64
    }

    fn extended_bar<F>(&mut self, i: usize, f: F) -> usize
    where
        F: Fn(usize) -> usize + Send + Sync + 'static,
    {
        f(i)
    }
}

#[allow(dead_code)]
/// Example async Extension trait
trait AsyncTestExt<T> {
    async fn extended_baz(&mut self, i: i32) -> f64;
}

impl<T> AsyncTestExt<T> for TestStruct<T>
where
    T: Clone + Debug + Send + Sync + 'static,
{
    async fn extended_baz(&mut self, i: i32) -> f64 {
        (i + 2) as f64
    }
}

#[allow(dead_code)]
#[derive(Clone)]
struct NonDebug;

#[actify]
impl NonDebug {
    fn foo(&self) {}
}

/// Argument names that collide with identifiers used in the generated code
#[allow(dead_code)]
#[derive(Clone, Debug)]
struct ShadowingActor {
    value: String,
}

#[actify]
impl ShadowingActor {
    fn store(&mut self, s: String, args: Vec<i32>, res: u8, result: bool) -> String {
        self.value = format!("{s}-{args:?}-{res}-{result}");
        self.value.clone()
    }
}

/// Types in scope whose names collide with what the generated code refers to.
///
/// A module of its own, since shadowing `Box` at file scope would break every
/// other test here.
///
/// Only the test build reaches it, hence the allowance.
#[allow(dead_code)]
mod shadowed_std_names {
    use actify::actify;

    #[allow(dead_code)]
    struct Box;

    #[allow(dead_code)]
    struct Any;

    #[derive(Clone, Debug)]
    pub struct ShadowedStdNames {
        pub value: i32,
    }

    #[actify]
    impl ShadowedStdNames {
        pub fn value(&self) -> i32 {
            self.value
        }
    }
}

/// A call's future has to be `Send` for a caller to spawn it, which the
/// generated method states by being an `async fn` on a concrete handle: the
/// compiler sees the future's real type and works `Send` out for itself.
///
/// Only the test build reaches it, hence the allowance.
#[allow(dead_code)]
mod send_calls {
    use actify::actify;

    #[derive(Clone, Debug)]
    pub struct Thermostat {
        pub celsius: i32,
    }

    #[actify]
    impl Thermostat {
        fn reading(&self) -> i32 {
            self.celsius
        }
    }

    /// Requires the call's future to be `Send`, spelled out rather than
    /// reached through `tokio::spawn`, which is where the requirement comes
    /// from in practice.
    pub async fn read(handle: &ThermostatHandle) -> i32 {
        fn require_send<F: Send>(future: F) -> F {
            future
        }

        require_send(handle.reading()).await
    }
}

/// A trait impl holding one method no actor call can express. Rust requires
/// every method of a trait in one impl block, so this cannot be split the way an
/// inherent impl can, and without `#[actify::skip]` the whole block fails.
#[derive(Clone, Debug)]
struct Ledger {
    entries: Vec<String>,
}

#[allow(dead_code)]
trait Merge {
    fn count(&self) -> usize;

    fn merge(&mut self, other: &Ledger);
}

#[allow(dead_code)]
#[actify]
impl Merge for Ledger {
    fn count(&self) -> usize {
        self.entries.len()
    }

    #[actify::skip]
    fn merge(&mut self, other: &Ledger) {
        self.entries.extend(other.entries.iter().cloned());
    }
}

/// Generic bounds written inline on the impl block instead of in a where clause
#[allow(dead_code)]
#[derive(Clone, Debug)]
struct InlineBounds<T> {
    value: T,
}

#[actify]
impl<T: Clone + Debug + Send + Sync + 'static> InlineBounds<T> {
    fn get_value(&self) -> T {
        self.value.clone()
    }
}

/// A self type whose generic argument is not the bare type parameter
#[allow(dead_code)]
#[derive(Clone, Debug)]
struct Wrapper<C> {
    items: C,
}

#[actify]
impl<T> Wrapper<Vec<T>>
where
    T: Clone + Debug + Send + Sync + 'static,
{
    fn first_item(&self) -> Option<T> {
        self.items.first().cloned()
    }
}

/// A const generic parameter on the impl block
#[allow(dead_code)]
#[derive(Clone, Debug)]
struct ConstActor<const N: usize> {
    data: [u8; N],
}

#[actify]
impl<const N: usize> ConstActor<N> {
    fn slots(&self) -> usize {
        N
    }
}

#[derive(Clone, Debug)]
struct ComplexActorTypes;

#[actify]
impl ComplexActorTypes {
    fn with_array(&self, data: [u8; 4]) -> u8 {
        data[0]
    }

    fn with_tuple(&self, pair: (String, i32)) -> String {
        format!("{}: {}", pair.0, pair.1)
    }

    fn with_fn_ptr(&self, f: fn(usize) -> usize, val: usize) -> usize {
        f(val)
    }

    fn with_trait_object(&self, handler: Box<dyn Fn(i32) -> i32 + Send + Sync>) -> i32 {
        handler(42)
    }

    fn with_destructure(&self, (a, b): (i32, i32)) -> i32 {
        a + b
    }

    fn with_mixed_destructure(&self, label: String, (x, y): (f64, f64)) -> String {
        format!("{}: ({}, {})", label, x, y)
    }
}

#[derive(Clone, Debug)]
struct AttributeTestActor;

#[allow(unused_variables)]
#[actify]
impl AttributeTestActor {
    /// Doc attribute propagated to handle trait
    fn with_doc(&self, x: i32) -> i32 {
        x
    }

    #[allow(unused_variables)]
    fn with_allow(&self, x: i32) -> i32 {
        42
    }

    #[allow(deprecated)]
    #[deprecated(note = "use with_doc instead")]
    fn with_deprecated(&self, x: i32) -> i32 {
        x
    }

    #[must_use]
    fn with_must_use(&self, x: i32) -> i32 {
        x + 1
    }

    #[cfg_attr(test, allow(unused_variables))]
    fn with_cfg_attr(&self, x: i32) -> i32 {
        42
    }

    #[cfg(target_os = "linux")]
    fn some_os_specific_method(&mut self) -> f64 {
        1.
    }

    #[cfg(target_os = "windows")]
    fn some_os_specific_method(&mut self) -> f64 {
        2.
    }
}

/// Two impl blocks for the same type, each behind a #[cfg(target_os)]. The
/// macro must put the same #[cfg] on everything it generates from a block:
/// compiling on Windows removes the Linux impl block, and generated code
/// without the gate would survive that, defining the handle trait twice and
/// forwarding to a method that was compiled out.
#[derive(Clone, Debug)]
struct CfgImplActor;

#[actify]
#[cfg(target_os = "linux")]
impl CfgImplActor {
    fn platform_value(&self) -> &'static str {
        "linux"
    }
}

#[actify]
#[cfg(target_os = "windows")]
impl CfgImplActor {
    fn platform_value(&self) -> &'static str {
        "windows"
    }
}

/// A non-Clone actor reached through a view, whose `&self` methods change the
/// state behind a lock.
#[derive(Debug)]
struct InteriorMutabilityActor {
    value: Mutex<i32>,
}

impl ToView<i32> for InteriorMutabilityActor {
    fn to_view(&self) -> i32 {
        *self.value.lock().unwrap()
    }
}

#[actify]
impl InteriorMutabilityActor {
    fn increment(&self) -> i32 {
        let mut value = self.value.lock().unwrap();
        *value += 1;
        *value
    }

    fn peek(&self) -> i32 {
        *self.value.lock().unwrap()
    }
}

/// Tests that proc-macro attributes like `#[instrument]` are stripped from generated
#[derive(Clone, Debug)]
struct InstrumentedActor {
    value: i32,
}

#[actify]
impl InstrumentedActor {
    #[tracing::instrument(skip_all)]
    fn get_value(&self) -> i32 {
        self.value
    }

    #[tracing::instrument(level = "debug", skip_all, fields(new_value))]
    fn set_value(&mut self, new_value: i32) {
        self.value = new_value;
    }

    /// Doc + instrument combined: doc propagates, instrument does not.
    #[tracing::instrument(skip_all)]
    async fn async_get(&self) -> i32 {
        self.value
    }

    #[tracing::instrument(skip_all)]
    #[allow(unused_variables)]
    fn unused_set(&mut self, v: i32) {
        self.value = v;
    }
}

/// Same test with unqualified `instrument` (via `use tracing::instrument`).
#[derive(Clone, Debug)]
struct UnqualifiedInstrumentActor {
    count: u32,
}

#[allow(unused_imports)]
use tracing::instrument;

#[actify]
impl UnqualifiedInstrumentActor {
    #[instrument(skip_all)]
    fn increment(&mut self) -> u32 {
        self.count += 1;
        self.count
    }

    #[instrument(level = "trace", skip_all)]
    fn get_count(&self) -> u32 {
        self.count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use actify::VecHandle;
    use std::sync::Mutex;
    use std::time::Duration;
    use tokio::time::{Instant, sleep};

    /// Generated code must not resolve `Box` or `Any` to a user type that
    /// happens to share the name.
    #[tokio::test]
    async fn test_shadowed_std_names() {
        use crate::shadowed_std_names::{ShadowedStdNames, ShadowedStdNamesHandle};

        let handle = ShadowedStdNamesHandle::new(ShadowedStdNames { value: 7 });

        assert_eq!(handle.value().await, 7);
    }

    /// The same reader runs against a real actor and against a hand-written
    /// stand-in, which is why it is generic over the trait rather than taking a
    /// `Handle`.
    #[tokio::test]
    async fn test_a_generic_caller_can_require_a_send_future() {
        use crate::send_calls::{Thermostat, ThermostatHandle, read};

        let handle = ThermostatHandle::new(Thermostat { celsius: 21 });

        assert_eq!(read(&handle).await, 21);
    }

    /// The skipped method stays on the type, the other one reaches the handle.
    #[tokio::test]
    async fn test_a_trait_impl_can_hold_a_skipped_method() {
        let mut ledger = Ledger {
            entries: vec!["a".to_string()],
        };
        ledger.merge(&Ledger {
            entries: vec!["b".to_string()],
        });

        assert_eq!(ledger.entries.len(), 2);

        let handle = LedgerHandle::new(ledger);

        assert_eq!(handle.count().await, 2);
    }

    /// An actor's methods all live in one block, so one handle carries them
    /// all, whether they read or write.
    #[tokio::test]
    async fn test_one_handle_carries_every_method_of_a_block() {
        let handle = SomeStructHandle::new(SomeStruct { inner_bool: false });

        handle.set_true().await;
        assert!(handle.get_inner().await);

        handle.set_false().await;
        assert!(!handle.get_inner().await);
    }

    // NOTE: "should not compile" tests live in tests/compile_fail/ and are run via trybuild.
    // A compile_error! from the macro fires at compile time, so it cannot be tested inline.

    #[tokio::test]
    async fn test_complex_arg_types() {
        let handle = ComplexActorTypesHandle::new(ComplexActorTypes);

        assert_eq!(handle.with_array([10, 20, 30, 40]).await, 10);
        assert_eq!(
            handle.with_tuple(("hello".to_string(), 42)).await,
            "hello: 42"
        );
        assert_eq!(handle.with_fn_ptr(|x| x * 2, 21).await, 42);
        assert_eq!(handle.with_trait_object(Box::new(|x| x * 3)).await, 126);
        assert_eq!(handle.with_destructure((3, 7)).await, 10);
        assert_eq!(
            handle
                .with_mixed_destructure("point".to_string(), (1.5, 2.5))
                .await,
            "point: (1.5, 2.5)"
        );
    }

    #[tokio::test]
    async fn test_attribute_propagation() {
        let handle = AttributeTestActorHandle::new(AttributeTestActor);

        // #[doc] is propagated to the handle trait; the call only needs to compile
        assert_eq!(handle.with_doc(5).await, 5);

        // #[allow(unused_variables)]: no warning despite the unused x
        assert_eq!(handle.with_allow(99).await, 42);

        // #[deprecated] is propagated to the handle trait and suppressed here
        #[allow(deprecated)]
        let result = handle.with_deprecated(10).await;
        assert_eq!(result, 10);

        // #[must_use] is propagated to the handle trait, but has no effect on
        // async fns: on those it warns about the unused Future, not the resolved
        // value, and .await always "uses" the Future
        assert_eq!(handle.with_must_use(5).await, 6);

        // #[cfg_attr(test, allow(unused_variables))]: no warning despite the unused x
        assert_eq!(handle.with_cfg_attr(99).await, 42);

        // #[cfg]: only the variant for this OS compiles
        #[cfg(target_os = "linux")]
        assert_eq!(handle.some_os_specific_method().await, 1.);
        #[cfg(target_os = "windows")]
        assert_eq!(handle.some_os_specific_method().await, 2.);

        // #[cfg] on the impl block gates all generated traits and impls
        let cfg_handle = CfgImplActorHandle::new(CfgImplActor);
        #[cfg(target_os = "linux")]
        assert_eq!(cfg_handle.platform_value().await, "linux");
        #[cfg(target_os = "windows")]
        assert_eq!(cfg_handle.platform_value().await, "windows");
    }

    #[tokio::test]
    async fn test_macro() {
        let actor_handle = TestStructHandle::new(TestStruct {
            inner_data: "Test".to_string(),
        });

        assert_eq!(actor_handle.foo(0, HashMap::new()).await, 1.);
        assert_eq!(actor_handle.baz(0).await, 2.);
    }

    /// Bounds written inline on the impl block must not leak into the generated
    /// trait reference or the call expression, where they are not valid syntax
    #[tokio::test]
    async fn test_inline_generic_bounds() {
        let handle = InlineBoundsHandle::new(InlineBounds { value: 7_i32 });
        assert_eq!(handle.get_value().await, 7);
    }

    /// The call must go through the full self type: `Wrapper<Vec<T>>`, not
    /// `Wrapper<T>` reconstructed from the impl block's generic parameters
    #[tokio::test]
    async fn test_non_identity_generic_argument() {
        let handle = WrapperHandle::new(Wrapper {
            items: vec![1_u8, 2, 3],
        });
        assert_eq!(handle.first_item().await, Some(1));
    }

    /// A `&'static` return outlives the actor, so it must keep working even
    /// though borrows of the actor's own state are rejected
    #[tokio::test]
    async fn test_static_reference_return() {
        let handle = CfgImplActorHandle::new(CfgImplActor);
        assert!(!handle.platform_value().await.is_empty());
    }

    #[tokio::test]
    async fn test_impl_level_const_generic() {
        let handle = ConstActorHandle::new(ConstActor { data: [0_u8; 4] });
        assert_eq!(handle.slots().await, 4);
    }

    /// Argument names like `s`, `args`, `res` and `result` must not clash with
    /// the identifiers the macro uses internally in the generated method body
    #[tokio::test]
    async fn test_shadowing_arg_names() {
        let handle = ShadowingActorHandle::new(ShadowingActor {
            value: String::new(),
        });

        let stored = handle.store("x".to_string(), vec![1, 2], 3, true).await;
        assert_eq!(stored, "x-[1, 2]-3-true");
    }

    /// An empty `#[actify]` block is how an actor asks for a handle carrying
    /// the built-in calls and nothing else, which is the only way to make an
    /// actor out of a type with no methods worth exposing.
    #[tokio::test]
    async fn test_an_empty_block_gives_a_handle_with_the_built_in_calls() {
        let handle = PlainHandle::new(Plain(1));

        assert_eq!(handle.get().await, Plain(1));

        handle.set(Plain(2)).await;
        assert_eq!(handle.get().await, Plain(2));
        assert_eq!(handle.read_handle().get().await, Plain(2));
    }

    /// An actor stops once the last handle to it goes out of scope, while one
    /// whose handle was cloned out of that scope keeps running.
    #[tokio::test]
    async fn test_handle_out_of_scope() {
        let baseline = alive_tasks();
        let handle_1 = PlainHandle::new(Plain(1));

        {
            let _handle_2 = PlainHandle::new(Plain(2));
            let _handle_3 = PlainHandle::new(Plain(3)); // These go out of scope
            let _handle_1_clone = handle_1.clone();
        }

        // Only handle_1's actor survives the scope
        let remaining = await_alive_tasks(baseline + 1).await;
        assert_eq!(
            remaining,
            baseline + 1,
            "expected only handle_1's actor to still be running"
        );
    }

    /// An actor runs one job at a time, so two actors calling each other each
    /// wait for a reply the other cannot produce yet. The crate docs state
    /// this; the test keeps that statement true.
    #[tokio::test(start_paused = true)]
    async fn test_actors_calling_each_other_never_complete() {
        let parser = ParserHandle::new(Parser { store: None });
        let store = StoreHandle::new(Store { parser: None });

        parser
            .set(Parser {
                store: Some(store.clone()),
            })
            .await;
        store
            .set(Store {
                parser: Some(parser.clone()),
            })
            .await;

        assert!(
            never_resolves(parser.parse()).await,
            "the cycle returned instead of blocking"
        );
    }

    #[tokio::test]
    async fn test_drain_vec() {
        let actor_handle = VecHandle::new(vec![1, 2, 3]);

        assert_eq!(actor_handle.drain(1..).await, vec![2, 3]);
        assert_eq!(actor_handle.get().await, vec![1]);
    }

    #[tokio::test]
    async fn test_instrument_attr_stripped_from_handle() {
        let handle = InstrumentedActorHandle::new(InstrumentedActor { value: 10 });

        assert_eq!(handle.get_value().await, 10);
        handle.set_value(42).await;
        assert_eq!(handle.get_value().await, 42);
        assert_eq!(handle.async_get().await, 42);
    }

    #[tokio::test]
    async fn test_unqualified_instrument_attr() {
        // Same test with unqualified `#[instrument]` (single-segment path)
        let handle = UnqualifiedInstrumentActorHandle::new(UnqualifiedInstrumentActor { count: 0 });

        assert_eq!(handle.increment().await, 1);
        assert_eq!(handle.increment().await, 2);
        assert_eq!(handle.get_count().await, 2);
    }

    /// A `&self` method behind interior mutability changes what a later read
    /// sees, which is why the view is read from the actor rather than cloned
    /// from it.
    #[tokio::test]
    async fn test_interior_mutability_is_visible_through_the_view() {
        let handle = InteriorMutabilityActorHandle::<i32>::new(InteriorMutabilityActor {
            value: Mutex::new(0),
        });

        assert_eq!(handle.peek().await, 0);
        assert_eq!(handle.get().await, 0);

        assert_eq!(handle.increment().await, 1);
        assert_eq!(handle.get().await, 1);
    }

    /// Returns whether a future is still pending once nothing else can make
    /// progress.
    ///
    /// The tests using this run on a paused clock, where tokio advances time
    /// as soon as every task is idle, so the timeout elapses immediately and
    /// its length is irrelevant.
    async fn never_resolves<F: std::future::Future>(future: F) -> bool {
        tokio::time::timeout(Duration::from_secs(1), future)
            .await
            .is_err()
    }

    fn alive_tasks() -> usize {
        tokio::runtime::Handle::current()
            .metrics()
            .num_alive_tasks()
    }

    /// Waits until the runtime reports `expected` alive tasks, or gives up.
    ///
    /// Tasks stop asynchronously: dropping a handle closes a channel, and the
    /// task only notices the next time it is scheduled. Sleeping for a fixed
    /// duration encodes a guess about how long that takes, which is what fails
    /// on a loaded machine. Polling returns as soon as the count settles and
    /// spends the whole deadline only when something is actually wrong, so the
    /// deadline can be generous without slowing the suite down.
    ///
    /// Returns the last observed count so the caller can assert on it and
    /// report the mismatch itself.
    async fn await_alive_tasks(expected: usize) -> usize {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let alive = alive_tasks();
            if alive == expected || Instant::now() >= deadline {
                return alive;
            }
            sleep(Duration::from_millis(5)).await;
        }
    }

    /// Gives tasks a chance to react, then reports the count.
    ///
    /// For assertions that a task keeps running, where polling for the
    /// expected value would return immediately and prove nothing. Waiting too
    /// briefly here can only make the test more lenient, never flaky.
    async fn settled_alive_tasks() -> usize {
        sleep(Duration::from_millis(50)).await;
        alive_tasks()
    }

    #[tokio::test]
    async fn test_handle_task_cleanup() {
        let baseline = alive_tasks();

        let handle = PlainHandle::new(Plain(42));

        let with_handle = await_alive_tasks(baseline + 1).await;
        assert!(
            with_handle > baseline,
            "Expected task count to increase after creating Handle. Baseline: {}, After: {}",
            baseline,
            with_handle
        );

        drop(handle);

        let after_drop = await_alive_tasks(baseline).await;
        assert_eq!(
            after_drop, baseline,
            "Expected task count to return to baseline after dropping Handle. Baseline: {}, After drop: {}",
            baseline, after_drop
        );
    }

    #[tokio::test]
    async fn test_handle_clone_task_cleanup() {
        let baseline = alive_tasks();

        let handle = PlainHandle::new(Plain(42));
        let handle_clone = handle.clone();

        let with_handles = await_alive_tasks(baseline + 1).await;
        assert_eq!(
            with_handles,
            baseline + 1,
            "Expected exactly one task for Handle and its clone. Baseline: {}, After: {}",
            baseline,
            with_handles
        );

        drop(handle);

        let after_first_drop = settled_alive_tasks().await;
        assert_eq!(
            after_first_drop,
            baseline + 1,
            "Task should still be running after dropping one clone. Baseline: {}, After: {}",
            baseline,
            after_first_drop
        );

        drop(handle_clone);

        let after_all_drop = await_alive_tasks(baseline).await;
        assert_eq!(
            after_all_drop, baseline,
            "Task should exit after all Handle clones are dropped. Baseline: {}, After: {}",
            baseline, after_all_drop
        );
    }

    #[tokio::test]
    async fn test_multiple_handles_task_cleanup() {
        let baseline = alive_tasks();

        let handle1 = PlainHandle::new(Plain(1));
        let handle2 = PlainHandle::new(Plain(2));
        let handle3 = PlainHandle::new(Plain(3));

        let with_handles = await_alive_tasks(baseline + 3).await;
        assert_eq!(
            with_handles,
            baseline + 3,
            "Expected three tasks for three Handles"
        );

        drop(handle1);
        assert_eq!(await_alive_tasks(baseline + 2).await, baseline + 2);

        drop(handle2);
        assert_eq!(await_alive_tasks(baseline + 1).await, baseline + 1);

        drop(handle3);
        assert_eq!(await_alive_tasks(baseline).await, baseline);
    }

    /// A ReadHandle holds a full handle internally, so it keeps the actor
    /// alive after the last Handle is dropped. The crate docs state this;
    /// the test keeps that statement true.
    #[tokio::test]
    async fn test_read_handle_keeps_actor_alive() {
        let baseline = alive_tasks();

        let handle = PlainHandle::new(Plain(1));
        let read_handle = handle.read_handle();

        let with_handle = await_alive_tasks(baseline + 1).await;
        assert_eq!(with_handle, baseline + 1, "Expected one task for Handle");

        drop(handle);

        let after_handle_drop = settled_alive_tasks().await;
        assert_eq!(
            after_handle_drop,
            baseline + 1,
            "The actor should stay alive while a ReadHandle exists"
        );

        assert_eq!(read_handle.get().await, Plain(1));

        drop(read_handle);

        let after_read_drop = await_alive_tasks(baseline).await;
        assert_eq!(
            after_read_drop, baseline,
            "The actor should stop once the last ReadHandle is dropped"
        );
    }
}

/// Two actors holding handles to each other, which is the shape that deadlocks.
#[allow(dead_code)]
#[derive(Clone, Debug)]
struct Parser {
    store: Option<StoreHandle>,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
struct Store {
    parser: Option<ParserHandle>,
}

#[actify]
impl Parser {
    async fn parse(&self) {
        self.store.as_ref().unwrap().save().await;
    }

    async fn is_ready(&self) -> bool {
        true
    }
}

#[actify]
impl Store {
    async fn save(&self) {
        self.parser.as_ref().unwrap().is_ready().await;
    }
}
