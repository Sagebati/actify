pub(crate) mod call;
pub(crate) mod handle;

use crate::parse::ImplInfo;
use quote::quote;

/// Generate all output code from the parsed IR.
pub fn generate(info: &ImplInfo) -> proc_macro2::TokenStream {
    let call_enum = call::generate(info);
    let handle_trait = handle::generate_trait(info);
    let handle_trait_impl = handle::generate_trait_impl(info);
    let original_impl = &info.original_impl;

    quote! {
        #call_enum
        #handle_trait
        #handle_trait_impl
        #original_impl
    }
}
