//! The plumbing behind a generated blocking handle.

use std::any::type_name;
use std::fmt::{self, Debug};
use std::marker::PhantomData;
use std::sync::Arc;

use super::Wait;
use super::channel::JobSender;
use super::message::Builtin;
use super::read_handle::ReadHandle;
use super::reply::{Answer, reply};
use crate::handles::ToView;

/// Panics because the actor is gone, whether it stopped or a method of it
/// panicked. Which of the two it was is on the actor's own exit event, at
/// ERROR level for a panic, because only the actor thread can tell them apart.
fn report_actor_gone<T>() -> ! {
    panic!("Actor of type {} is no longer running", type_name::<T>());
}

/// The plumbing behind a generated blocking handle: a shared sending half, how
/// to wait for a reply, and the calls every actor answers.
///
/// `#[actify(blocking)]` generates a handle of the actor's own that wraps this
/// one, so a caller never names it. Cloning it shares access to the same actor
/// across threads.
pub struct Handle<T, V, M, S = DefaultSender<M>> {
    // The `Arc` is what stops the actor: the last handle to drop drops the
    // only sending half with it.
    sender: Arc<S>,
    // A word beside the pointer, rather than a fourth type parameter on every
    // generated handle. The branch it costs is on a path that is about to
    // block or spin for far longer.
    wait: Wait,
    actor: Marker<T, V, M>,
}

/// Names the types a handle is for without holding one of them, and without
/// borrowing their auto traits: a handle is `Send` and `Sync` on the strength
/// of its channel alone.
type Marker<T, V, M> = PhantomData<fn() -> (T, V, M)>;

/// The channel a blocking handle uses when the caller supplies none: an
/// unbounded [`std::sync::mpsc`] queue, so a call never waits to be queued.
pub type DefaultSender<M> = std::sync::mpsc::Sender<M>;

/// The receiving half of [`DefaultSender`], which the actor thread reads.
pub type DefaultReceiver<M> = std::sync::mpsc::Receiver<M>;

impl<T, V, M, S> Clone for Handle<T, V, M, S> {
    fn clone(&self) -> Self {
        Handle {
            sender: Arc::clone(&self.sender),
            wait: self.wait,
            actor: PhantomData,
        }
    }
}

impl<T, V, M, S> Debug for Handle<T, V, M, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let actor = type_name::<T>();
        let view = type_name::<V>();
        if actor == view {
            write!(f, "Handle<{actor}>")
        } else {
            write!(f, "Handle<{actor}, {view}>")
        }
    }
}

impl<T, V, M, S> Handle<T, V, M, S> {
    pub(super) fn from_sender(sender: S, wait: Wait) -> Self {
        Handle {
            sender: Arc::new(sender),
            wait,
            actor: PhantomData,
        }
    }

    /// Returns a [`ReadHandle`] that provides read-only access to this actor.
    pub fn read_handle(&self) -> ReadHandle<T, V, M, S> {
        ReadHandle::new(self.clone())
    }
}

impl<T, V, M, S> Handle<T, V, M, S>
where
    T: ToView<V> + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
    M: From<Builtin<T, V>> + Send + 'static,
    S: JobSender<M>,
{
    /// Returns the actor's current view.
    ///
    /// # Panics
    ///
    /// Panics if the actor has stopped, either because one of its methods
    /// panicked or because its thread ended. See [Actor lifetime and
    /// panics](crate#actor-lifetime-and-panics).
    pub fn get(&self) -> V {
        let (reply, answer) = reply();
        self.__call(Builtin::Get(reply).into(), answer)
    }

    /// Overwrites the inner value of the actor with the new value.
    ///
    /// # Panics
    ///
    /// Panics if the actor has stopped.
    pub fn set(&self, val: T) {
        let (reply, answer) = reply();
        self.__call(Builtin::Set(val, reply).into(), answer)
    }
}

impl<T, V, M, S> Handle<T, V, M, S>
where
    T: 'static,
    M: Send + 'static,
    S: JobSender<M>,
{
    /// Queues one call and blocks until its reply arrives.
    ///
    /// The reply channel is the caller's to make, so that it carries the
    /// method's own return type rather than something erased.
    #[doc(hidden)]
    pub fn __call<R>(&self, message: M, answer: Answer<R>) -> R {
        if self.sender.send(message).is_ok() {
            if let Ok(res) = answer.recv(self.wait) {
                return res;
            }
        }
        report_actor_gone::<T>()
    }
}

#[cfg(test)]
mod tests {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::mpsc::sync_channel;

    use super::super::Wait;
    use crate::actify;

    #[derive(Clone, Debug, PartialEq)]
    struct Counter(i32);

    #[actify(blocking)]
    impl Counter {
        fn add(&mut self, value: i32) -> i32 {
            self.0 += value;
            self.0
        }

        fn boom(&self) {
            panic!("{SYNC_PANIC_PAYLOAD}")
        }
    }

    const SYNC_PANIC_PAYLOAD: &str = "the actor's own panic message";

    /// A handle is a pointer and a word saying how to wait: the `Wait` is a
    /// byte, which pads out to a word beside the pointer. That is the whole
    /// cost of the choice being a value rather than a type parameter, and the
    /// async handle's one word is what it is measured against.
    #[test]
    fn test_a_handle_is_a_pointer_and_a_wait() {
        assert_eq!(size_of::<CounterHandle>(), 2 * size_of::<usize>());
    }

    /// Nothing runs until the caller puts the actor on a thread, so a handle
    /// whose actor was never started is a handle to nothing.
    #[test]
    fn test_a_built_actor_serves_only_once_it_is_running() {
        let (handle, actor) = CounterHandle::builder(Counter(0)).build();
        drop(actor);

        let called = catch_unwind(AssertUnwindSafe(|| handle.add(1)));
        assert!(called.is_err(), "the actor was never started");
    }

    /// A panicking method takes the actor's thread with it, and the caller
    /// waiting on that call learns the actor is gone rather than waiting
    /// forever. Both wait modes have to notice.
    #[test]
    fn test_a_panicking_method_reaches_its_caller() {
        for wait in [Wait::Park, Wait::Spin] {
            let (handle, actor) = CounterHandle::builder(Counter(0)).wait(wait).build();
            let running = std::thread::spawn(actor);

            let called = catch_unwind(AssertUnwindSafe(|| handle.boom()));
            let payload = called.expect_err("the call panics");
            let message = payload
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| payload.downcast_ref::<&str>().copied())
                .expect("a string payload");
            // The actor's own payload stays on the actor's thread: the caller
            // is told the actor is gone, not what it was doing.
            assert!(message.contains("no longer running"), "{wait:?}: {message}");
            assert!(!message.contains(SYNC_PANIC_PAYLOAD), "{wait:?}: {message}");

            assert!(running.join().is_err(), "{wait:?}: the thread unwound");
        }
    }

    /// One method panicking ends the actor for every handle on it, not only
    /// for the caller that tripped it.
    #[test]
    fn test_a_panic_kills_the_actor_for_every_clone() {
        let (handle, actor) = CounterHandle::builder(Counter(0)).build();
        let running = std::thread::spawn(actor);
        let other = handle.clone();

        let _ = catch_unwind(AssertUnwindSafe(|| handle.boom()));
        let _ = running.join();

        let called = catch_unwind(AssertUnwindSafe(|| other.add(1)));
        assert!(called.is_err(), "the actor is gone for this handle too");
    }

    /// A caller that panics while its call is in flight is the blocking
    /// analogue of a dropped call future: the actor answers into a slot nobody
    /// is holding, which must not stop it.
    #[test]
    fn test_a_panicking_caller_does_not_stop_the_actor() {
        let (handle, actor) = CounterHandle::builder(Counter(0)).build();
        let running = std::thread::spawn(actor);

        let caller = handle.clone();
        let panicked = std::thread::spawn(move || {
            caller.add(1);
            panic!("the caller gives up");
        });
        assert!(panicked.join().is_err(), "the caller panicked");

        assert_eq!(handle.add(1), 2, "the actor is still serving");

        drop(handle);
        running.join().unwrap();
    }

    /// A bounded channel makes a caller wait to be queued, which is the
    /// backpressure it was chosen for. The default one never does.
    #[test]
    fn test_a_bounded_channel_still_answers_every_call() {
        let (handle, actor) = CounterHandle::builder(Counter(0))
            .channel(sync_channel(1))
            .build();
        let running = std::thread::spawn(actor);

        for expected in 1..=50 {
            assert_eq!(handle.add(1), expected);
        }

        drop(handle);
        running.join().unwrap();
    }

    #[test]
    fn test_a_read_handle_reads_what_was_set() {
        let (handle, actor) = CounterHandle::builder(Counter(0)).build();
        let running = std::thread::spawn(actor);
        let reader = handle.read_handle();

        handle.set(Counter(9));
        assert_eq!(reader.get(), Counter(9));

        drop(handle);
        drop(reader);
        running.join().unwrap();
    }

    /// A read handle holds a sending half too, so the actor outlives the
    /// writing handle it came from.
    #[test]
    fn test_a_read_handle_keeps_the_actor_alive() {
        let (handle, actor) = CounterHandle::builder(Counter(4)).build();
        let running = std::thread::spawn(actor);
        let reader = handle.read_handle();

        drop(handle);
        assert_eq!(reader.get(), Counter(4));

        drop(reader);
        running.join().unwrap();
    }

    #[test]
    fn test_debug_names_the_view_only_when_it_differs() {
        let (handle, actor) = CounterHandle::builder(Counter(0)).build();
        let running = std::thread::spawn(actor);

        assert_eq!(
            format!("{handle:?}"),
            format!("CounterHandle<{}>", std::any::type_name::<Counter>())
        );

        drop(handle);
        running.join().unwrap();
    }
}
