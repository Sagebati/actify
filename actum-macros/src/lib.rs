//! Procedural macros for this repository's `actum` crate, a hard fork of
//! [actify](https://github.com/AvalorAI/actify).
//!
//! Everything here is re-exported from `actum`, and the generated code refers
//! to `actum`'s types, so the macros only work alongside that crate.
#![warn(missing_docs)]

mod codegen;
mod parse;

use proc_macro::TokenStream;

/// Leaves one method off the generated handle.
///
/// ```ignore
/// #[actum]
/// impl Store {
///     fn len(&self) -> usize { self.entries.len() }
///
///     #[actum::skip]
///     fn merge(&mut self, other: &Store) { .. }
/// }
/// ```
///
/// The method stays on the type and is unchanged. It is not validated, so it
/// may take or return references, which an actor call cannot express.
///
/// Expands to nothing: `#[actum]` reads it and strips it from the output.
#[proc_macro_attribute]
pub fn skip(_args: TokenStream, input: TokenStream) -> TokenStream {
    input
}

/// Parsed arguments from `#[actum(...)]`.
struct ActumArgs {
    custom_name: Option<syn::LitStr>,
    /// `#[actum(blocking)]`: the actor runs on a thread, not on a future.
    blocking: bool,
}

impl syn::parse::Parse for ActumArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut args = ActumArgs {
            custom_name: None,
            blocking: false,
        };

        while !input.is_empty() {
            let ident: syn::Ident = input.parse()?;
            if ident == "name" {
                input.parse::<syn::Token![=]>()?;
                let name: syn::LitStr = input.parse()?;
                args.custom_name = Some(name);
            } else if ident == "blocking" {
                args.blocking = true;
            } else {
                return Err(syn::Error::new_spanned(
                    ident,
                    "unknown actum attribute; expected `blocking` or `name = \"...\"`",
                ));
            }

            if !input.is_empty() {
                input.parse::<syn::Token![,]>()?;
            }
        }

        Ok(args)
    }
}

/// Emit diagnostics together with the impl block they came from.
///
/// Returning only the errors would delete every method of the type, so each
/// call site would report a further "no method named ..." error and bury the
/// diagnostic that actually explains the problem.
fn report(error: syn::Error, impl_block: &syn::ItemImpl) -> TokenStream {
    let compile_errors = error.to_compile_error();
    quote::quote! {
        #compile_errors
        #impl_block
    }
    .into()
}

/// Expands an impl block so its methods can be called remotely through a
/// handle.
///
/// Generates a message enum with one variant per method, a handle whose
/// methods mirror the block's and send those variants, and a builder for it.
/// Arguments and return values keep their types the whole way, so a call is
/// checked as if it were a direct one.
///
/// # `#[actum(blocking)]`
///
/// Generates the same three things without `async`: the actor's loop is a
/// plain `fn` that a [`std::thread`] runs, and the handle's methods block until
/// the actor answers. An `async fn` in such a block is an error, since there is
/// no runtime to drive it. See the `blocking` module for what a caller chooses
/// between parking and busy-waiting.
#[proc_macro_attribute]
pub fn actum(attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut impl_block = syn::parse_macro_input!(item as syn::ItemImpl);

    let args = match syn::parse::<ActumArgs>(attr) {
        Ok(args) => args,
        Err(error) => return report(error, &impl_block),
    };

    let backend = if args.blocking {
        parse::Backend::Blocking
    } else {
        parse::Backend::Async
    };

    match parse::ImplInfo::from_impl_block(&mut impl_block, args.custom_name, backend) {
        Ok(info) => codegen::generate(&info).into(),
        Err(error) => report(error, &impl_block),
    }
}
