//! The message enum an actor is served, and everything that runs it.
//!
//! A call travels as one of these variants, carrying its arguments by value and
//! the caller's reply channel, so nothing on the way to the actor is boxed or
//! downcast.

use super::backend;
use crate::parse::{ImplInfo, MethodInfo};
use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::{Attribute, GenericParam, Ident};

/// The method attributes that belong on a variant and its arms.
///
/// Only the conditional ones: a variant that a `#[cfg]` removes has to lose its
/// match arms with it. A doc comment belongs on the variant alone, and rustdoc
/// warns about one on a match arm.
fn cfg_attrs(method: &MethodInfo) -> Vec<&Attribute> {
    method
        .attributes
        .iter()
        .filter(|attr| attr.path().is_ident("cfg") || attr.path().is_ident("cfg_attr"))
        .collect()
}

/// The view type parameter the generated enum declares.
pub fn view_param() -> Ident {
    Ident::new("__ActifyV", Span::call_site())
}

/// How a variant reaches the method it stands for.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Carrier {
    /// The variant's arm names the method directly.
    Direct,
    /// The variant carries a function pointer to it.
    ///
    /// A method's own `where` clause is in scope where the call is made and
    /// not inside the single `dispatch` that runs every variant, which is
    /// bound by the impl block alone. A non-capturing closure written at the
    /// call site coerces to the pointer, which costs a word in the message
    /// and an inlinable indirect call.
    Thunk,
}

/// How a method's call reaches it.
///
/// The parser has already refused anything neither shape can carry, so every
/// method that reaches here is one or the other.
pub fn carrier(method: &MethodInfo) -> Carrier {
    let bounded = method
        .method_generics
        .where_clause
        .as_ref()
        .is_some_and(|clause| !clause.predicates.is_empty());
    if bounded {
        Carrier::Thunk
    } else {
        Carrier::Direct
    }
}

/// The function-pointer field a thunked variant carries.
pub fn thunk_type(method: &MethodInfo, impl_type: &syn::Type) -> TokenStream {
    let arg_types: Vec<_> = method.arg_types.iter().collect();
    let output = &method.output_type;
    let mutability = method.is_mutable.then(|| quote! { mut });
    quote! { fn(&#mutability #impl_type #(, #arg_types)*) -> #output }
}

/// The variant a method's call travels as, e.g. `say_hi` becomes `SayHi`.
pub fn variant_ident(method: &MethodInfo) -> Ident {
    let pascal: String = method
        .ident
        .to_string()
        .split('_')
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect();
    Ident::new(&pascal, method.ident.span())
}

/// Names the generated enum with a view type the caller chooses, as in
/// `GreeterCall<__V>`.
pub fn call_type(info: &ImplInfo, view: &Ident) -> TokenStream {
    let call = &info.call_enum_ident;
    let params = param_names(info);
    quote! { #call<#(#params,)* #view> }
}

/// The impl block's parameters by name alone, for naming a type that takes them.
pub fn param_names(info: &ImplInfo) -> Vec<TokenStream> {
    info.generics
        .params
        .iter()
        .map(|param| match param {
            GenericParam::Type(ty) => {
                let ident = &ty.ident;
                quote! { #ident }
            }
            GenericParam::Const(konst) => {
                let ident = &konst.ident;
                quote! { #ident }
            }
            GenericParam::Lifetime(lifetime) => {
                let lifetime = &lifetime.lifetime;
                quote! { #lifetime }
            }
        })
        .collect()
}

/// The impl block's parameters as a declaration, with every bound stripped.
///
/// Bounds belong on the impls, not on the type: an enum that demanded them
/// would force every mention of it to prove them, including in a signature that
/// only names the type.
pub fn declared_params(info: &ImplInfo) -> Vec<TokenStream> {
    info.generics
        .params
        .iter()
        .map(|param| match param {
            GenericParam::Type(ty) => {
                let ident = &ty.ident;
                quote! { #ident }
            }
            GenericParam::Const(konst) => {
                let ident = &konst.ident;
                let ty = &konst.ty;
                quote! { const #ident: #ty }
            }
            GenericParam::Lifetime(lifetime) => {
                let lifetime = &lifetime.lifetime;
                quote! { #lifetime }
            }
        })
        .collect()
}

/// The name of the generated loop, e.g. `__actify_run_greeter_call`.
pub fn run_ident(info: &ImplInfo) -> Ident {
    let call = &info.call_enum_ident;
    Ident::new(
        &format!("__actify_run_{}", call.to_string().to_lowercase()),
        call.span(),
    )
}

/// Generates the message enum, the glue that lets it carry a builtin, a
/// `Debug` that names variants without asking anything of their fields, and
/// the loop that serves the actor.
pub fn generate(info: &ImplInfo) -> TokenStream {
    let attrs = &info.attributes;
    let call = &info.call_enum_ident;
    let run_ident = run_ident(info);
    let impl_type = &info.impl_type;
    let view = view_param();

    let declared = declared_params(info);
    let call_ty = call_type(info, &view);

    let (_, _, where_clause) = info.generics.split_for_impl();

    // The impls take the block's own parameters plus the view, which the enum
    // declares with a default so `GreeterCall` alone still names something.
    let mut with_view = info.generics.clone();
    with_view.params.push(syn::parse_quote!(#view));
    let (impl_generics, _, _) = with_view.split_for_impl();

    let root = backend::root(info);
    let asyncness = backend::asyncness(info);
    let wait_param = backend::wait_param(info);
    let recv = backend::recv(info);

    // The loop is a free function rather than a method, so nothing the actor
    // type declares has to be a trait implementation.
    let mut run_generics = info.generics.clone();
    run_generics
        .params
        .push(syn::parse_quote!(#view: Send + 'static));
    run_generics
        .params
        .push(syn::parse_quote!(__ActifyRx: #root::JobReceiver<#call_ty>));
    run_generics
        .make_where_clause()
        .predicates
        .push(syn::parse_quote!(#impl_type: ::actify::ToView<#view> + Send + Sync + 'static));
    let (run_generics_impl, _, run_where) = run_generics.split_for_impl();

    let variants = info.methods.iter().map(|method| {
        let variant = variant_ident(method);
        let arg_names: Vec<_> = method.arg_names.iter().collect();
        let arg_types: Vec<_> = method.arg_types.iter().collect();
        let output = &method.output_type;
        let doc = format!("The `{}` call.", method.ident);
        let attrs = cfg_attrs(method);
        let thunk = (carrier(method) == Carrier::Thunk).then(|| {
            let ty = thunk_type(method, impl_type);
            quote! { __actify_thunk: #ty, }
        });
        quote! {
            #[doc = #doc]
            #(#attrs)*
            #variant {
                #(#arg_names: #arg_types,)*
                #thunk
                __actify_reply: #root::__private::Reply<#output>,
            },
        }
    });

    let arms = info.methods.iter().map(|method| {
        let variant = variant_ident(method);
        let ident = &method.ident;
        let arg_names: Vec<_> = method.arg_names.iter().collect();
        let output = &method.output_type;
        let prefix = super::handle::build_call_prefix(info);
        let mutability = method.is_mutable.then(|| quote! { mut });
        let awaiter = method.is_async.then(|| quote! { .await });
        let attrs = cfg_attrs(method);
        let thunked = carrier(method) == Carrier::Thunk;
        let binding = thunked.then(|| quote! { __actify_thunk, });
        let invocation = if thunked {
            quote! { __actify_thunk(&#mutability __actify_actor.inner #(, #arg_names)*) }
        } else {
            quote! { #prefix::#ident(&#mutability __actify_actor.inner #(, #arg_names)*)#awaiter }
        };
        quote! {
            #(#attrs)*
            #call::#variant { #(#arg_names,)* #binding __actify_reply } => {
                let __actify_result: #output = #invocation;
                __actify_actor.respond(__actify_reply, __actify_result);
            }
        }
    });

    let debug_arms = info.methods.iter().map(|method| {
        let variant = variant_ident(method);
        let name = variant.to_string();
        let attrs = cfg_attrs(method);
        quote! {
            #(#attrs)*
            #call::#variant { .. } => #name,
        }
    });

    let enum_doc = format!(
        "The message a call on a `{}` actor travels to it as.\n\n\
         Name it to give a channel its item type. Every variant carries its \
         call's arguments and the caller's reply channel, so nothing on the \
         way is boxed.",
        quote! { #impl_type },
    );
    let call_name = call.to_string();

    quote! {
        #[doc = #enum_doc]
        #(#attrs)*
        #[allow(non_camel_case_types)]
        pub enum #call<#(#declared,)* #view = #impl_type> {
            /// One of the calls every handle has, whatever its actor declares.
            __ActifyBuiltin(#root::Builtin<#impl_type, #view>),
            #(#variants)*
        }

        #(#attrs)*
        impl #impl_generics ::std::convert::From<#root::Builtin<#impl_type, #view>>
            for #call_ty #where_clause
        {
            fn from(__actify_builtin: #root::Builtin<#impl_type, #view>) -> Self {
                #call::__ActifyBuiltin(__actify_builtin)
            }
        }

        #(#attrs)*
        impl #impl_generics ::std::fmt::Debug for #call_ty #where_clause {
            fn fmt(&self, __actify_f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                let __actify_variant = match self {
                    #call::__ActifyBuiltin(_) => "Builtin",
                    #(#debug_arms)*
                };
                ::std::write!(__actify_f, "{}::{}", #call_name, __actify_variant)
            }
        }

        #(#attrs)*
        // A deprecated method still has to run when it is called, and the call
        // is the macro's, not the caller's: the warning belongs at their call
        // site, which the handle's method carries it to.
        #[allow(deprecated)]
        #[doc(hidden)]
        pub #asyncness fn #run_ident #run_generics_impl(
            mut __actify_rx: __ActifyRx,
            mut __actify_actor: #root::__private::Actor<#impl_type>,
            #wait_param
        ) #run_where {
            while let Some(__actify_call) = #recv {
                match __actify_call {
                    #call::__ActifyBuiltin(__actify_builtin) => {
                        #root::__private::run_builtin(&mut __actify_actor, __actify_builtin)
                    }
                    #(#arms)*
                }
            }
        }
    }
}
