//! What the two backends spell differently.
//!
//! The async and the blocking actor share the message enum, the handle's
//! shape, the thunk machinery and every validation rule. What separates them
//! fits in this file: which module the library items come from, whether a
//! signature says `async`, whether a call site says `.await`, what a built
//! actor is, and whether `new` needs a runtime to exist.

use crate::parse::{Backend, ImplInfo};
use proc_macro2::TokenStream;
use quote::quote;

/// The library items the generated code names.
///
/// Both backends expose the same names, so a path is the module and nothing
/// else: `::actum::Handle` against `::actum::blocking::Handle`.
pub fn root(info: &ImplInfo) -> TokenStream {
    match info.backend {
        Backend::Async => quote! { ::actum },
        Backend::Blocking => quote! { ::actum::blocking },
    }
}

/// `async`, on a signature that returns a future.
pub fn asyncness(info: &ImplInfo) -> Option<TokenStream> {
    (info.backend == Backend::Async).then(|| quote! { async })
}

/// `.await`, on a call that returns a future.
pub fn awaiter(info: &ImplInfo) -> Option<TokenStream> {
    (info.backend == Backend::Async).then(|| quote! { .await })
}

/// What a built-but-not-yet-running actor is.
///
/// A future the caller spawns on an executor, or a closure the caller puts on
/// a thread. Both are `Send` and owe nothing to their builder.
pub fn actor_type(info: &ImplInfo) -> TokenStream {
    match info.backend {
        Backend::Async => quote! { impl ::std::future::Future<Output = ()> + Send },
        Backend::Blocking => quote! { impl ::std::ops::FnOnce() + Send + 'static },
    }
}

/// Whether `Handle::new` can exist: it spawns, and only one of the two
/// backends needs a dependency to do that.
///
/// The async one spawns on Tokio, so it exists only where actum can, which it
/// says with its own `tokio` feature. A thread needs nothing.
pub fn can_spawn(info: &ImplInfo) -> bool {
    match info.backend {
        Backend::Async => cfg!(feature = "tokio"),
        Backend::Blocking => true,
    }
}

/// How `Handle::new` describes itself.
pub fn new_doc(info: &ImplInfo) -> &'static str {
    match info.backend {
        Backend::Async => {
            " Builds the actor, spawns it on Tokio and returns its handle.\n\n\
             The spelling of [`builder`](Self::builder) followed by a\n\
             `tokio::spawn`. Turn actum's `tokio` feature off and the caller\n\
             spawns the actor future instead."
        }
        Backend::Blocking => {
            " Builds the actor, puts it on a thread and returns its handle.\n\n\
             The spelling of [`builder`](Self::builder) followed by a\n\
             `std::thread::spawn`. The thread is detached, so it ends when the\n\
             last handle does; use [`builder`](Self::builder) to keep its\n\
             `JoinHandle`."
        }
    }
}

/// How a handle's methods take the handle.
///
/// An async handle sends through a [`Sink`], which wants `&mut self`, and that
/// is also what keeps one task at a time in front of the sender's single waker
/// slot. A blocking handle's channel sends through `&self` and parks the
/// calling thread, so it needs neither.
///
/// [`Sink`]: https://docs.rs/futures-sink/latest/futures_sink/trait.Sink.html
pub fn receiver(info: &ImplInfo) -> TokenStream {
    match info.backend {
        Backend::Async => quote! { &mut self },
        Backend::Blocking => quote! { &self },
    }
}

/// What the generated handle's sender parameter has to be.
///
/// An async handle sends through any [`Sink`] over its message type; a
/// blocking one through the blocking `JobSender`. Both are cloned to clone
/// the handle, so both ask for `Clone` here.
///
/// [`Sink`]: https://docs.rs/futures-sink/latest/futures_sink/trait.Sink.html
pub fn sender_bound(info: &ImplInfo, call_ty: &TokenStream) -> TokenStream {
    match info.backend {
        Backend::Async => quote! { ::actum::Sink<#call_ty> + Unpin + ::std::clone::Clone },
        Backend::Blocking => {
            quote! { ::actum::blocking::JobSender<#call_ty> + ::std::clone::Clone }
        }
    }
}

/// What the loop's receiver parameter has to be.
///
/// The async loop reads any [`Stream`] of its message type, and is spawned,
/// so the stream has to travel with it. The blocking loop reads the blocking
/// `JobReceiver`.
///
/// [`Stream`]: https://docs.rs/futures-core/latest/futures_core/trait.Stream.html
pub fn receiver_bound(info: &ImplInfo, call_ty: &TokenStream) -> TokenStream {
    match info.backend {
        Backend::Async => {
            quote! { ::actum::Stream<Item = #call_ty> + Unpin + Send + 'static }
        }
        Backend::Blocking => quote! { ::actum::blocking::JobReceiver<#call_ty> },
    }
}

/// What the actor type has to be for this backend to serve it.
///
/// Both are moved to where they run, so both are `Send + 'static`. An async
/// method borrows `&self` across an `await`, which puts `&T` in a future that
/// has to be `Send`, so the async actor is `Sync` as well. A thread never
/// holds such a borrow.
pub fn actor_bound(info: &ImplInfo) -> TokenStream {
    match info.backend {
        Backend::Async => quote! { Send + Sync + 'static },
        Backend::Blocking => quote! { Send + 'static },
    }
}

/// The extra argument a blocking loop takes: how it waits for its next job.
///
/// The async loop has no equivalent, because waiting for a future is the
/// executor's business rather than the loop's.
pub fn wait_param(info: &ImplInfo) -> Option<TokenStream> {
    (info.backend == Backend::Blocking).then(|| quote! { __actum_wait: ::actum::blocking::Wait })
}

/// Takes the next job, however this backend waits for one.
pub fn recv(info: &ImplInfo) -> TokenStream {
    match info.backend {
        Backend::Async => quote! { ::actum::__private::next(&mut __actum_rx).await },
        Backend::Blocking => {
            quote! { ::actum::blocking::__private::next(&mut __actum_rx, __actum_wait) }
        }
    }
}
