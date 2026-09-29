mod backend;
mod call;
mod handle;
mod handle_struct;

use crate::parse::ImplInfo;
use quote::quote;

/// Generate all output code from the parsed IR.
pub fn generate(info: &ImplInfo) -> proc_macro2::TokenStream {
    let call_enum = call::generate(info);
    let handle = handle_struct::generate(info);
    let original_impl = &info.original_impl;

    quote! {
        #call_enum
        #handle
        #original_impl
    }
}
