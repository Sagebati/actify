//! Asserts on the tracing instrumentation, by rendering it with a fmt
//! subscriber and matching on the output lines.
//!
//! These tests have their own binary on purpose. tracing caches per-callsite
//! interest process-wide, and the first thread to hit a callsite decides that
//! cache: a thread without a subscriber caches the callsite as disabled for
//! every later subscriber. In this binary every test installs a subscriber, so
//! the cache is always decided by a thread that wants the output. A test added
//! here must install one too.

use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use actum::actum;

/// An actor whose methods do the two things these tests need from inside the
/// actor task: emit an event, and panic.
#[derive(Clone, Debug)]
struct Probe;

#[actum]
impl Probe {
    fn log(&self, message: String) {
        tracing::info!("{message}");
    }

    fn boom(&mut self) {
        panic!("boom")
    }
}
use tracing_subscriber::fmt::MakeWriter;

/// Sets a TRACE-level fmt subscriber for this thread and returns its output.
///
/// The guard must be bound to a name, as `let _ = ...` drops it immediately.
fn capture() -> (tracing::subscriber::DefaultGuard, Buffer) {
    let buffer = Buffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::level_filters::LevelFilter::TRACE)
        .with_writer(buffer.clone())
        .finish();
    (tracing::subscriber::set_default(subscriber), buffer)
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

#[tokio::test]
async fn test_actor_methods_run_inside_the_actor_span() {
    let (_guard, output) = capture();

    let mut handle = ProbeHandle::new(Probe);
    handle.log("emitted by an actor method".to_string()).await;

    let output = output.contents();
    let line = output
        .lines()
        .find(|line| line.contains("emitted by an actor method"))
        .expect("the event is captured");
    // The prefix only: the span's identity fields are pinned by their own
    // tests.
    let span = format!("actor{{actor_type=\"{}\"", std::any::type_name::<Probe>());
    assert!(line.contains(&span), "no actor span on: {line}");
}

/// Two actors of the same type are told apart on the span itself: each has
/// its own actor_id and its own spawn site.
#[tokio::test]
async fn test_same_type_actors_are_distinguishable_on_the_span() {
    let (_guard, output) = capture();

    let mut first = ProbeHandle::new(Probe);
    let mut second = ProbeHandle::new(Probe); // Its own line, so its own spawn site

    first.log("first probe".to_string()).await;
    second.log("second probe".to_string()).await;

    let output = output.contents();
    let first_line = find_line(&output, "first probe");
    let second_line = find_line(&output, "second probe");

    // Only distinctness is stable: the counter is process-wide and tests
    // run in parallel.
    assert_ne!(parse_actor_id(first_line), parse_actor_id(second_line));

    let first_site = parse_spawned_at(first_line);
    let second_site = parse_spawned_at(second_line);
    // The file name only: path separators differ across platforms.
    assert!(
        first_site.contains("tracing.rs"),
        "not this file: {first_site}"
    );
    assert!(
        second_site.contains("tracing.rs"),
        "not this file: {second_site}"
    );
    assert_ne!(
        first_site, second_site,
        "each Handle::new line is its own spawn site"
    );
}

/// The exit event names the instance itself, not only through the span: the
/// `log` bridge drops span fields, so events must carry the id too.
#[tokio::test]
async fn test_the_exit_event_names_the_actor_instance() {
    let (_guard, output) = capture();

    let handle = ProbeHandle::new(Probe);
    drop(handle);
    tokio::task::yield_now().await;

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

/// A panicking method is the exit a subscriber must not miss: the std panic
/// hook prints to stderr, which never reaches a structured log pipeline.
#[tokio::test]
async fn test_a_panicking_actor_reports_its_exit_as_an_error() {
    let (_guard, output) = capture();

    let handle = ProbeHandle::new(Probe);
    let mut caller = handle.clone();
    let _ = tokio::spawn(async move { caller.boom().await }).await;

    let output = output.contents();
    let line = output
        .lines()
        .find(|line| line.contains("Actor stopped"))
        .expect("the exit is reported");
    assert!(line.contains("ERROR"), "wrong level on: {line}");
    assert!(line.contains("reason=Panicked"), "no reason on: {line}");
    let actor_type = format!("actor_type=\"{}\"", std::any::type_name::<Probe>());
    assert!(line.contains(&actor_type), "no actor type on: {line}");
}

/// Dropping every handle ends the actor task without unwinding, which is an
/// unremarkable exit and reports at DEBUG.
#[tokio::test]
async fn test_a_dropped_actor_reports_its_exit() {
    let (_guard, output) = capture();

    let handle = ProbeHandle::new(Probe);
    drop(handle);
    tokio::task::yield_now().await;

    let output = output.contents();
    let line = output
        .lines()
        .find(|line| line.contains("Actor stopped"))
        .expect("the exit is reported");
    assert!(line.contains("DEBUG"), "wrong level on: {line}");
    assert!(line.contains("reason=Stopped"), "no reason on: {line}");
}
