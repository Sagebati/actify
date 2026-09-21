//! The handle an actor gets of its own.
//!
//! Where a trait on the library's generic handle could only ever send what that
//! handle already carried, a struct of the actor's own fixes its message type
//! to the generated enum. That is what lets a call travel as data.

use crate::parse::{ImplInfo, MethodInfo};
use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::Ident;

use super::backend;
use super::call::{
    Carrier, call_type, carrier, declared_params, param_names, run_ident, variant_ident,
};

/// The sender parameter the generated handle declares.
fn sender() -> Ident {
    Ident::new("__ActumS", Span::call_site())
}

/// Generates the handle struct, its constructors, and one method per actified
/// method.
pub fn generate(info: &ImplInfo) -> TokenStream {
    let attrs = &info.attributes;
    let handle = &info.handle_trait_ident;
    let impl_type = &info.impl_type;
    let sender = sender();
    let call_ty = call_type(info);
    let root = backend::root(info);

    let declared = declared_params(info);
    let names = param_names(info);
    let handle_ty = quote! { #handle<#(#names,)* #sender> };
    let inner = quote! { #root::Handle<#impl_type, #call_ty, #sender> };

    // Naming the type takes the parameters alone; only the impls carry bounds.
    let mut bare = info.generics.clone();
    bare.params.push(syn::parse_quote!(#sender));
    let (bare_generics, _, _) = bare.split_for_impl();

    // A handle is cloned by cloning its sending half, so it is `Clone` exactly
    // when that is.
    let mut cloneable = info.generics.clone();
    cloneable
        .params
        .push(syn::parse_quote!(#sender: ::std::clone::Clone));
    let (clone_generics, _, _) = cloneable.split_for_impl();

    // What every call on this handle needs: a channel that carries this
    // actor's messages, and an actor the backend can serve.
    let sender_bound = backend::sender_bound(info, &call_ty);
    let actor_bound = backend::actor_bound(info);
    let mut full = info.generics.clone();
    full.params.push(syn::parse_quote!(#sender: #sender_bound));
    full.make_where_clause()
        .predicates
        .push(syn::parse_quote!(#impl_type: #actor_bound));
    let (full_generics, _, full_where) = full.split_for_impl();

    // The constructors live where the sender is already decided, so that
    // `Handle::new(val)` has nothing left to infer.
    let mut made = info.generics.clone();
    made.make_where_clause()
        .predicates
        .push(syn::parse_quote!(#impl_type: #actor_bound));
    let (made_generics, _, made_where) = made.split_for_impl();
    let made_ty = quote! { #handle<#(#names,)* #root::DefaultSender<#call_ty>> };

    let constructors = constructors(info);
    let builder = generate_builder(info);
    let methods = info.methods.iter().map(|m| method(m, info, &call_ty));

    let doc = format!(
        "A handle to a `{}` actor.\n\n\
         Cloning it shares access to the same actor. A method sends its call as \
         a variant of the actor's own message enum, so nothing on the way to \
         the actor is boxed.",
        super::handle::display_type(info),
    );
    let debug_name = handle.to_string();

    quote! {
        #[doc = #doc]
        #(#attrs)*
        pub struct #handle <#(#declared,)* #sender = #root::DefaultSender<#call_ty>>(
            #inner
        );

        #(#attrs)*
        impl #clone_generics ::std::clone::Clone for #handle_ty {
            fn clone(&self) -> Self {
                #handle(::std::clone::Clone::clone(&self.0))
            }
        }

        #(#attrs)*
        impl #bare_generics ::std::fmt::Debug for #handle_ty {
            fn fmt(&self, __actum_f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                ::std::write!(
                    __actum_f,
                    "{}<{}>",
                    #debug_name,
                    ::std::any::type_name::<#impl_type>()
                )
            }
        }

        #(#attrs)*
        impl #made_generics #made_ty #made_where {
            #constructors
        }

        #(#attrs)*
        impl #full_generics #handle_ty #full_where {
            #(#methods)*
        }

        #builder
    }
}

/// The ways a handle is made.
fn constructors(info: &ImplInfo) -> TokenStream {
    let handle = &info.handle_trait_ident;
    let impl_type = &info.impl_type;
    let root = backend::root(info);

    let run = run_ident(info);

    // `new` spawns, so it exists only where this backend can: a thread always,
    // Tokio only where actum has it, which actum says with its own feature.
    let new_doc = backend::new_doc(info);
    let new = backend::can_spawn(info).then(|| {
        quote! {
            #[doc = #new_doc]
            #[track_caller]
            pub fn new(__actum_val: #impl_type) -> Self {
                #handle(#root::__private::spawn(__actum_val, #run))
            }
        }
    });

    let builder = builder_ident(info);
    let names = param_names(info);
    let builder_doc = match info.backend {
        crate::parse::Backend::Async => {
            " Starts building a handle whose actor future the caller spawns.\n\n\
             `build` hands back this handle and that future, so the caller\n\
             chooses the executor."
        }
        crate::parse::Backend::Blocking => {
            " Starts building a handle whose actor the caller runs.\n\n\
             `build` hands back this handle and a closure, so the caller\n\
             chooses the thread and keeps its `JoinHandle`. It is also where\n\
             `wait` chooses between parking and busy-waiting."
        }
    };
    quote! {
        #new

        #[doc = #builder_doc]
        #[track_caller]
        pub fn builder(__actum_val: #impl_type) -> #builder<#(#names,)*> {
            #builder(#root::__private::builder(__actum_val))
        }
    }
}

/// The name of the generated builder, e.g. `GreeterHandleBuilder`.
fn builder_ident(info: &ImplInfo) -> Ident {
    let handle = &info.handle_trait_ident;
    Ident::new(&format!("{handle}Builder"), handle.span())
}

/// Generates the builder, which is the library's with this actor's handle in
/// place of the generic one.
fn generate_builder(info: &ImplInfo) -> TokenStream {
    let attrs = &info.attributes;
    let run = run_ident(info);
    let handle = &info.handle_trait_ident;
    let builder = builder_ident(info);
    let impl_type = &info.impl_type;
    let call_ty = call_type(info);
    let root = backend::root(info);
    let actor_type = backend::actor_type(info);
    let sender_bound = backend::sender_bound(info, &call_ty);
    let receiver_bound = backend::receiver_bound(info, &call_ty);
    let actor_bound = backend::actor_bound(info);

    let declared = declared_params(info);
    let names = param_names(info);
    let channel = Ident::new("__ActumC", Span::call_site());
    let builder_ty = quote! { #builder<#(#names,)* #channel> };

    let mut common = info.generics.clone();
    common
        .make_where_clause()
        .predicates
        .push(syn::parse_quote!(#impl_type: #actor_bound));

    let default_channel = common.clone();
    let (default_generics, _, default_where) = default_channel.split_for_impl();
    let default_ty = quote! { #builder<#(#names,)* #root::DefaultChannel> };

    let mut own_channel = common.clone();
    own_channel
        .params
        .push(syn::parse_quote!(__ActumTx: #sender_bound));
    own_channel
        .params
        .push(syn::parse_quote!(__ActumRx: #receiver_bound));
    let (own_generics, _, own_where) = own_channel.split_for_impl();
    let own_ty = quote! { #builder<#(#names,)* (__ActumTx, __ActumRx)> };

    let mut bare = info.generics.clone();
    bare.params.push(syn::parse_quote!(#channel));
    let (bare_generics, _, _) = bare.split_for_impl();
    let bare_generics_tokens = quote! { #bare_generics };

    let doc = format!(
        "Builds a handle to a `{}` actor, and the {} that serves it.",
        super::handle::display_type(info),
        match info.backend {
            crate::parse::Backend::Async => "future",
            crate::parse::Backend::Blocking => "closure",
        },
    );
    let debug_name = builder.to_string();
    let wait = wait_method(info, &bare_generics_tokens, &builder_ty);
    let build_doc = match info.backend {
        crate::parse::Backend::Async => {
            " Returns the handle and the future that serves it.\n\n\
             Nothing runs until the caller polls that future."
        }
        crate::parse::Backend::Blocking => {
            " Returns the handle and the closure that serves it.\n\n\
             Nothing runs until the caller calls it, which is usually a\n\
             `std::thread::spawn`."
        }
    };

    quote! {
        #[doc = #doc]
        #(#attrs)*
        pub struct #builder<
            #(#declared,)*
            #channel = #root::DefaultChannel,
        >(#root::HandleBuilder<#impl_type, #call_ty, #channel>);

        #(#attrs)*
        impl #bare_generics ::std::fmt::Debug for #builder_ty {
            fn fmt(&self, __actum_f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                ::std::write!(__actum_f, "{}", #debug_name)
            }
        }

        #wait

        #(#attrs)*
        impl #default_generics #default_ty #default_where {
            /// Serves the actor through a channel the caller owns, in place of
            /// the default unbounded one.
            ///
            /// A bounded channel is how backpressure is asked for.
            pub fn channel<__ActumTx, __ActumRx>(
                self,
                __actum_channel: (__ActumTx, __ActumRx),
            ) -> #builder<#(#names,)* (__ActumTx, __ActumRx)>
            where
                __ActumTx: #sender_bound,
                __ActumRx: #receiver_bound,
            {
                #builder(self.0.channel(__actum_channel))
            }

            #[doc = #build_doc]
            pub fn build(
                self,
            ) -> (
                #handle<#(#names,)* #root::DefaultSender<#call_ty>>,
                #actor_type,
            ) {
                let (__actum_handle, __actum_actor) = self.0.build(#run);
                (#handle(__actum_handle), __actum_actor)
            }
        }

        #(#attrs)*
        impl #own_generics #own_ty #own_where {
            #[doc = #build_doc]
            pub fn build(
                self,
            ) -> (
                #handle<#(#names,)* __ActumTx>,
                #actor_type,
            ) {
                let (__actum_handle, __actum_actor) = self.0.build(#run);
                (#handle(__actum_handle), __actum_actor)
            }
        }
    }
}

/// The blocking builder's choice of how both ends wait.
///
/// The async backend has no equivalent: waiting for a future is the executor's
/// business rather than the actor's.
fn wait_method(
    info: &ImplInfo,
    bare_generics: &TokenStream,
    builder_ty: &TokenStream,
) -> Option<TokenStream> {
    let attrs = &info.attributes;
    (info.backend == crate::parse::Backend::Blocking).then(|| {
        quote! {
            #(#attrs)*
            impl #bare_generics #builder_ty {
                /// Chooses how the actor waits for jobs and how a caller waits
                /// for replies.
                ///
                /// [`Park`](::actum::blocking::Wait::Park) by default;
                /// [`Spin`](::actum::blocking::Wait::Spin) busy-waits.
                pub fn wait(self, __actum_wait: ::actum::blocking::Wait) -> Self {
                    Self(self.0.wait(__actum_wait))
                }
            }
        }
    })
}

/// One method on the generated handle.
fn method(method: &MethodInfo, info: &ImplInfo, call_ty: &TokenStream) -> TokenStream {
    let root = backend::root(info);
    let asyncness = backend::asyncness(info);
    let awaiter = backend::awaiter(info);
    let receiver = backend::receiver(info);
    // `#[deprecated]` and `#[must_use]` belong here, on the method a caller
    // reaches for, which is now the only place they are written.
    let attrs = &method.attributes;
    let ident = &method.ident;
    let arg_names: Vec<_> = method.arg_names.iter().collect();
    let arg_types: Vec<_> = method.arg_types.iter().collect();
    let method_generics = &method.method_generics;
    let where_clause = &method.method_generics.where_clause;
    let output = &method.output_type;
    let return_type = super::handle::quote_return_type(output);

    let variant = variant_ident(method);
    let call = &info.call_enum_ident;

    // A method's own `where` clause is in scope here and not inside the
    // actor's `dispatch`, so the call carries a pointer to the method rather
    // than having `dispatch` name it. The closure captures nothing, which is
    // what lets it coerce to a plain function pointer.
    let thunked = carrier(method) == Carrier::Thunk;
    let thunk = thunked.then(|| {
        let prefix = super::handle::build_call_prefix(info);
        quote! {
            __actum_thunk: |__actum_inner #(, #arg_names)*| {
                #prefix::#ident(__actum_inner #(, #arg_names)*)
            },
        }
    });

    // The thunk names the method, and a deprecated one warns there even
    // though the forwarder around it is deprecated too. A direct forwarder
    // never names its method - only the loop does - so only a thunked one
    // needs the allowance.
    let allow_deprecated = thunked.then(|| quote! { #[allow(deprecated)] });

    quote! {
        #(#attrs)*
        #allow_deprecated
        pub #asyncness fn #ident #method_generics(#receiver, #(#arg_names: #arg_types),*) #return_type
        #where_clause
        {
            let (__actum_reply, __actum_rx) = #root::__private::reply();
            let __actum_call: #call_ty = #call::#variant {
                #(#arg_names,)*
                #thunk
                __actum_reply,
            };
            self.0.__call(__actum_call, __actum_rx) #awaiter
        }
    }
}
