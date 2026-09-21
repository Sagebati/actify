//! Helpers shared by the generated message enum and the generated handle.

use crate::parse::ImplInfo;
use quote::quote;

/// Build the fully qualified syntax prefix for calling the original method.
/// This is the same for every method in the impl block:
/// - Direct impl: `<Type>`
/// - Trait impl: `<Type as Trait>`
///
/// The self type is used exactly as written, so `impl<T> Wrapper<Vec<T>>` calls
/// `<Wrapper<Vec<T>>>::method`. Rebuilding it from the impl block's parameter
/// list would instead produce `Wrapper::<T>`, which names a different type.
pub(crate) fn build_call_prefix(info: &ImplInfo) -> proc_macro2::TokenStream {
    let impl_type = &info.impl_type;

    match &info.trait_path {
        None => quote! { <#impl_type> },
        Some(path) => quote! { <#impl_type as #path> },
    }
}

/// Quote a return type, omitting the `->` for unit returns.
pub(crate) fn quote_return_type(ty: &syn::Type) -> proc_macro2::TokenStream {
    match ty {
        syn::Type::Tuple(tuple) if tuple.elems.is_empty() => quote! {},
        _ => quote! { -> #ty },
    }
}

/// The impl type as a reader would write it, for generated docs.
///
/// `quote!` prints every token with a space around it, so `Fixed<N>` comes out
/// as `Fixed < N >`. This closes the gaps a person would not leave.
pub(crate) fn display_type(info: &ImplInfo) -> String {
    let impl_type = &info.impl_type;
    let spaced = quote! { #impl_type }.to_string();
    spaced
        .replace(" < ", "<")
        .replace(" >", ">")
        .replace("< ", "<")
        .replace(" ,", ",")
        .replace("& ", "&")
        .replace(" ::", "::")
        .replace(":: ", "::")
}
