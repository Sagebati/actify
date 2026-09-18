//! Procedural macros for the [actify](https://docs.rs/actify) crate.
//!
//! Everything here is re-exported from `actify`, and the generated code refers
//! to `actify`'s types, so the macros only work alongside that crate.
#![warn(missing_docs)]

mod codegen;
mod parse;

use proc_macro::TokenStream;

/// Leaves one method off the generated handle.
///
/// ```ignore
/// #[actify]
/// impl Store {
///     fn len(&self) -> usize { self.entries.len() }
///
///     #[actify::skip]
///     fn merge(&mut self, other: &Store) { .. }
/// }
/// ```
///
/// The method stays on the type and is unchanged. It is not validated, so it
/// may take or return references, which an actor call cannot express.
///
/// Expands to nothing: `#[actify]` reads it and strips it from the output.
#[proc_macro_attribute]
pub fn skip(_args: TokenStream, input: TokenStream) -> TokenStream {
    input
}

/// Parsed arguments from `#[actify(...)]`.
struct ActifyArgs {
    custom_name: Option<syn::LitStr>,
}

impl syn::parse::Parse for ActifyArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut args = ActifyArgs { custom_name: None };

        while !input.is_empty() {
            let ident: syn::Ident = input.parse()?;
            if ident == "name" {
                input.parse::<syn::Token![=]>()?;
                let name: syn::LitStr = input.parse()?;
                args.custom_name = Some(name);
            } else {
                return Err(syn::Error::new_spanned(
                    ident,
                    "unknown actify attribute; expected `name = \"...\"`",
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
#[proc_macro_attribute]
pub fn actify(attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut impl_block = syn::parse_macro_input!(item as syn::ItemImpl);

    let args = match syn::parse::<ActifyArgs>(attr) {
        Ok(args) => args,
        Err(error) => return report(error, &impl_block),
    };

    match parse::ImplInfo::from_impl_block(&mut impl_block, args.custom_name) {
        Ok(info) => codegen::generate(&info).into(),
        Err(error) => report(error, &impl_block),
    }
}
