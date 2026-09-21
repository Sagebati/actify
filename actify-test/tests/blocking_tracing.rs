//! Asserts on the tracing instrumentation of a blocking actor.
//!
//! The async backend hangs the `actor` span on the loop's future with
//! `Instrument`; a blocking actor enters it around the loop instead. These
//! tests pin that the two are indistinguishable from the outside.
//!
//! Like `tracing.rs`, these have their own binary: tracing caches per-callsite
//! interest process-wide, and the first thread to reach a callsite decides that
//! cache. A test added here must install a subscriber too.
//!
//! The actor really is on another thread here, and a thread-local subscriber
//! does not reach it. So instead of `Handle::new`, which spawns its own thread,
//! these build the actor and run it under the test's own dispatcher, which is
//! what [`run`] does. A real program sets a global subscriber and needs none of
//! this.

use std::io::{self, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use actify::actify;
use tracing::Dispatch;
use tracing_subscriber::fmt::MakeWriter;

/// An actor whose methods do the two things these tests need from inside the
/// actor thread: emit an event, and panic.
#[derive(Clone, Debug)]
struct Probe;

#[actify(blocking)]
impl Probe {
    fn log(&self, message: String) {
        tracing::info!("{message}");
    }

    fn boom(&mut self) {
        panic!("boom")
    }
}

/// Sets a TRACE-level fmt subscriber for this thread and returns its output.
///
/// The guard must be bound to a name, as `let _ = ...` drops it immediately.
/// The [`Dispatch`] comes back too, so [`run`] can give the actor's thread the
/// same subscriber.
fn capture() -> (tracing::subscriber::DefaultGuard, Dispatch, Buffer) {
    let buffer = Buffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::level_filters::LevelFilter::TRACE)
        .with_writer(buffer.clone())
        .finish();
    let dispatch = Dispatch::new(subscriber);
    let guard = tracing::dispatcher::set_default(&dispatch);
    (guard, dispatch, buffer)
}

/// Puts an actor on a thread that reports to the test's own subscriber.
fn run(actor: impl FnOnce() + Send + 'static, dispatch: &Dispatch) -> JoinHandle<()> {
    let dispatch = dispatch.clone();
    std::thread::spawn(move || tracing::dispatcher::with_default(&dispatch, actor))
}

/// Routes the fmt subscriber's output into a shared string.
#[derive(Clone, Default)]
struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Buffer {
    fn contents(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl Write for Buffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Buffer {
    type Writer = Buffer;

    fn make_writer(&'a self) -> Buffer {
        self.clone()
    }
}

fn find_line<'a>(output: &'a str, needle: &str) -> &'a str {
    output
        .lines()
        .find(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("no line contains {needle:?} in: {output}"))
}

/// Parses the first `actor_id=` value on a rendered line or section.
fn parse_actor_id(line: &str) -> u64 {
    let digits: String = line
        .split("actor_id=")
        .nth(1)
        .unwrap_or_else(|| panic!("no actor_id on: {line}"))
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits
        .parse()
        .unwrap_or_else(|_| panic!("no numeric actor_id on: {line}"))
}

/// Parses the `spawned_at=` value out of the rendered span section, which
/// closes with a brace right after it.
fn parse_spawned_at(line: &str) -> &str {
    line.split("spawned_at=")
        .nth(1)
        .and_then(|rest| rest.split('}').next())
        .unwrap_or_else(|| panic!("no spawned_at on: {line}"))
}

/// The one thing a blocking backend is most likely to get wrong, since it
/// holds a span guard where the async one instruments a future.
#[test]
fn test_actor_methods_run_inside_the_actor_span() {
    let (_guard, dispatch, output) = capture();

    let (handle, actor) = ProbeHandle::builder(Probe).build();
    let running = run(actor, &dispatch);

    handle.log("emitted by an actor method".to_string());
    drop(handle);
    running.join().unwrap();

    let output = output.contents();
    let line = find_line(&output, "emitted by an actor method");
    // The prefix only: the span's identity fields are pinned by their own
    // test.
    let span = format!("actor{{actor_type=\"{}\"", std::any::type_name::<Probe>());
    assert!(line.contains(&span), "no actor span on: {line}");
}

/// Two actors of the same type are told apart on the span itself: each has its
/// own actor_id and its own spawn site, which is the `#[track_caller]` chain
/// through the generated builder.
#[test]
fn test_same_type_actors_are_distinguishable_on_the_span() {
    let (_guard, dispatch, output) = capture();

    let (first, first_actor) = ProbeHandle::builder(Probe).build();
    let (second, second_actor) = ProbeHandle::builder(Probe).build(); // Its own line

    let first_running = run(first_actor, &dispatch);
    let second_running = run(second_actor, &dispatch);

    first.log("first probe".to_string());
    second.log("second probe".to_string());

    drop(first);
    drop(second);
    first_running.join().unwrap();
    second_running.join().unwrap();

    let output = output.contents();
    let first_line = find_line(&output, "first probe");
    let second_line = find_line(&output, "second probe");

    // Only distinctness is stable: the counter is process-wide and tests run
    // in parallel.
    assert_ne!(parse_actor_id(first_line), parse_actor_id(second_line));

    let first_site = parse_spawned_at(first_line);
    let second_site = parse_spawned_at(second_line);
    // The file name only: path separators differ across platforms.
    assert!(
        first_site.contains("blocking_tracing.rs"),
        "not this file: {first_site}"
    );
    assert_ne!(
        first_site, second_site,
        "each builder line is its own spawn site"
    );
}

/// The exit event names the instance itself, not only through the span: the
/// `log` bridge drops span fields, so events must carry the id too.
#[test]
fn test_the_exit_event_names_the_actor_instance() {
    let (_guard, dispatch, output) = capture();

    let (handle, actor) = ProbeHandle::builder(Probe).build();
    let running = run(actor, &dispatch);
    drop(handle);
    running.join().unwrap();

    let output = output.contents();
    let line = find_line(&output, "Actor stopped");
    // Parsed after the message, so the span's own actor_id cannot satisfy
    // this: the event field itself must name the actor.
    let event_fields = line
        .split("Actor stopped")
        .nth(1)
        .expect("the message is on the line");
    assert_eq!(parse_actor_id(event_fields), parse_actor_id(line));
}

/// A panicking method unwinds the actor's thread, which is the exit a
/// subscriber must not miss: the std panic hook prints to stderr, which never
/// reaches a structured log pipeline.
#[test]
fn test_a_panicking_actor_reports_its_exit_as_an_error() {
    let (_guard, dispatch, output) = capture();

    let (handle, actor) = ProbeHandle::builder(Probe).build();
    let running = run(actor, &dispatch);

    // The actor is gone before it answers, so the call panics too.
    let called = catch_unwind(AssertUnwindSafe(|| handle.boom()));
    assert!(called.is_err(), "the caller learns the actor is gone");
    assert!(running.join().is_err(), "the actor's thread unwound");

    let output = output.contents();
    let line = find_line(&output, "Actor stopped");
    assert!(line.contains("ERROR"), "wrong level on: {line}");
    assert!(line.contains("reason=Panicked"), "no reason on: {line}");
    let actor_type = format!("actor_type=\"{}\"", std::any::type_name::<Probe>());
    assert!(line.contains(&actor_type), "no actor type on: {line}");
}

/// Dropping every handle ends the actor's loop without unwinding, which is an
/// unremarkable exit and reports at DEBUG.
#[test]
fn test_a_dropped_actor_reports_its_exit() {
    let (_guard, dispatch, output) = capture();

    let (handle, actor) = ProbeHandle::builder(Probe).build();
    let running = run(actor, &dispatch);
    drop(handle);
    running.join().unwrap();

    let output = output.contents();
    let line = find_line(&output, "Actor stopped");
    assert!(line.contains("DEBUG"), "wrong level on: {line}");
    assert!(line.contains("reason=Stopped"), "no reason on: {line}");
}
