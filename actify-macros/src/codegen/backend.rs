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
/// else: `::actify::Handle` against `::actify::blocking::Handle`.
pub fn root(info: &ImplInfo) -> TokenStream {
    match info.backend {
        Backend::Async => quote! { ::actify },
        Backend::Blocking => quote! { ::actify::blocking },
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
/// The async one spawns on Tokio, so it exists only where actify can, which it
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
             `tokio::spawn`. Turn actify's `tokio` feature off and the caller\n\
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

/// The extra argument a blocking loop takes: how it waits for its next job.
///
/// The async loop has no equivalent, because waiting for a future is the
/// executor's business rather than the loop's.
pub fn wait_param(info: &ImplInfo) -> Option<TokenStream> {
    (info.backend == Backend::Blocking).then(|| quote! { __actify_wait: ::actify::blocking::Wait })
}

/// Takes the next job, however this backend waits for one.
pub fn recv(info: &ImplInfo) -> TokenStream {
    match info.backend {
        Backend::Async => quote! { ::actify::JobReceiver::recv(&mut __actify_rx).await },
        Backend::Blocking => {
            quote! { ::actify::blocking::__private::next(&mut __actify_rx, __actify_wait) }
        }
    }
}
