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

/// The view parameter the generated handle declares.
fn view() -> Ident {
    Ident::new("__ActifyV", Span::call_site())
}

/// The sender parameter the generated handle declares.
fn sender() -> Ident {
    Ident::new("__ActifyS", Span::call_site())
}

/// Generates the handle struct, its constructors, the calls every handle has,
/// and one method per actified method.
pub fn generate(info: &ImplInfo) -> TokenStream {
    let attrs = &info.attributes;
    let handle = &info.handle_trait_ident;
    let impl_type = &info.impl_type;
    let view = view();
    let sender = sender();
    let call_ty = call_type(info, &view);
    let root = backend::root(info);
    let asyncness = backend::asyncness(info);
    let awaiter = backend::awaiter(info);

    let declared = declared_params(info);
    let names = param_names(info);
    let handle_ty = quote! { #handle<#(#names,)* #view, #sender> };
    let inner = quote! { #root::Handle<#impl_type, #view, #call_ty, #sender> };

    // Naming the type takes the parameters alone; only the impls carry bounds.
    let mut bare = info.generics.clone();
    bare.params.push(syn::parse_quote!(#view));
    bare.params.push(syn::parse_quote!(#sender));
    let (bare_generics, _, _) = bare.split_for_impl();

    // What every call on this handle needs: a view the actor can produce, and
    // a channel that carries this actor's messages.
    let mut full = info.generics.clone();
    full.params
        .push(syn::parse_quote!(#view: ::std::clone::Clone + Send + Sync + 'static));
    full.params
        .push(syn::parse_quote!(#sender: #root::JobSender<#call_ty>));
    full.make_where_clause()
        .predicates
        .push(syn::parse_quote!(#impl_type: ::actify::ToView<#view> + Send + Sync + 'static));
    let (full_generics, _, full_where) = full.split_for_impl();

    // The constructors live where the sender is already decided, so that
    // `Handle::new(val)` has nothing left to infer.
    let mut made = info.generics.clone();
    made.params
        .push(syn::parse_quote!(#view: ::std::clone::Clone + Send + Sync + 'static));
    made.make_where_clause()
        .predicates
        .push(syn::parse_quote!(#impl_type: ::actify::ToView<#view> + Send + Sync + 'static));
    let (made_generics, _, made_where) = made.split_for_impl();
    let made_ty = quote! { #handle<#(#names,)* #view, #root::DefaultSender<#call_ty>> };

    let constructors = constructors(info);
    let builder = generate_builder(info);
    let methods = info.methods.iter().map(|m| method(m, info, &call_ty));

    let doc = format!(
        "A handle to a `{}` actor.\n\n\
         Cloning it shares access to the same actor. A method sends its call as \
         a variant of the actor's own message enum, so nothing on the way to \
         the actor is boxed.",
        quote! { #impl_type },
    );
    let debug_name = handle.to_string();

    quote! {
        #[doc = #doc]
        #(#attrs)*
        pub struct #handle <#(#declared,)* #view = #impl_type, #sender = #root::DefaultSender<#call_ty>>(
            #inner
        );

        #(#attrs)*
        impl #bare_generics ::std::clone::Clone for #handle_ty {
            fn clone(&self) -> Self {
                #handle(::std::clone::Clone::clone(&self.0))
            }
        }

        #(#attrs)*
        impl #bare_generics ::std::fmt::Debug for #handle_ty {
            fn fmt(&self, __actify_f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                // The view is named only when it differs, so a log line says
                // which of two handles on the same actor it came from.
                let __actify_actor = ::std::any::type_name::<#impl_type>();
                let __actify_view = ::std::any::type_name::<#view>();
                if __actify_actor == __actify_view {
                    ::std::write!(__actify_f, "{}<{}>", #debug_name, __actify_actor)
                } else {
                    ::std::write!(
                        __actify_f,
                        "{}<{}, {}>",
                        #debug_name,
                        __actify_actor,
                        __actify_view
                    )
                }
            }
        }

        #(#attrs)*
        impl #bare_generics #handle_ty {
            /// Wraps a handle built through [`builder`](Self::builder).
            pub fn from_handle(__actify_handle: #inner) -> Self {
                #handle(__actify_handle)
            }
        }

        #(#attrs)*
        impl #made_generics #made_ty #made_where {
            #constructors
        }

        #(#attrs)*
        #[allow(unused_parens, private_interfaces, deprecated)]
        impl #full_generics #handle_ty #full_where {
            /// Returns the actor's current view.
            ///
            /// # Panics
            ///
            /// Panics if the actor has stopped.
            pub #asyncness fn get(&self) -> #view {
                self.0.get() #awaiter
            }

            /// Overwrites the actor's value.
            ///
            /// # Panics
            ///
            /// Panics if the actor has stopped.
            pub #asyncness fn set(&self, __actify_val: #impl_type) {
                self.0.set(__actify_val) #awaiter
            }

            /// Returns a read-only handle to the same actor.
            pub fn read_handle(
                &self,
            ) -> #root::ReadHandle<#impl_type, #view, #call_ty, #sender> {
                self.0.read_handle()
            }

            #(#methods)*
        }

        #builder
    }
}

/// The ways a handle is made.
fn constructors(info: &ImplInfo) -> TokenStream {
    let handle = &info.handle_trait_ident;
    let impl_type = &info.impl_type;
    let view = view();
    let root = backend::root(info);

    let run = run_ident(info);

    // `new` spawns, so it exists only where this backend can: a thread always,
    // Tokio only where actify has it, which actify says with its own feature.
    let new_doc = backend::new_doc(info);
    let new = backend::can_spawn(info).then(|| {
        quote! {
            #[doc = #new_doc]
            #[track_caller]
            pub fn new(__actify_val: #impl_type) -> Self {
                #handle(#root::__private::spawn(__actify_val, #run))
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
        pub fn builder(__actify_val: #impl_type) -> #builder<#(#names,)* #view> {
            #builder(#root::__private::builder(__actify_val))
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
    let view = view();
    let call_ty = call_type(info, &view);
    let root = backend::root(info);
    let actor_type = backend::actor_type(info);

    let declared = declared_params(info);
    let names = param_names(info);
    let channel = Ident::new("__ActifyC", Span::call_site());
    let builder_ty = quote! { #builder<#(#names,)* #view, #channel> };

    let mut common = info.generics.clone();
    common
        .params
        .push(syn::parse_quote!(#view: ::std::clone::Clone + Send + Sync + 'static));
    common
        .make_where_clause()
        .predicates
        .push(syn::parse_quote!(#impl_type: ::actify::ToView<#view> + Send + Sync + 'static));

    let default_channel = common.clone();
    let (default_generics, _, default_where) = default_channel.split_for_impl();
    let default_ty = quote! { #builder<#(#names,)* #view, #root::DefaultChannel> };

    let mut own_channel = common.clone();
    own_channel
        .params
        .push(syn::parse_quote!(__ActifyTx: #root::JobSender<#call_ty>));
    own_channel
        .params
        .push(syn::parse_quote!(__ActifyRx: #root::JobReceiver<#call_ty>));
    let (own_generics, _, own_where) = own_channel.split_for_impl();
    let own_ty = quote! { #builder<#(#names,)* #view, (__ActifyTx, __ActifyRx)> };

    let mut bare = info.generics.clone();
    bare.params.push(syn::parse_quote!(#view));
    bare.params.push(syn::parse_quote!(#channel));
    let (bare_generics, _, _) = bare.split_for_impl();
    let bare_generics_tokens = quote! { #bare_generics };

    let doc = format!(
        "Builds a handle to a `{}` actor, and the {} that serves it.",
        quote! { #impl_type },
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
            #view = #impl_type,
            #channel = #root::DefaultChannel,
        >(#root::HandleBuilder<#impl_type, #view, #call_ty, #channel>);

        #(#attrs)*
        impl #bare_generics ::std::fmt::Debug for #builder_ty {
            fn fmt(&self, __actify_f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                ::std::write!(__actify_f, "{}", #debug_name)
            }
        }

        #wait

        #(#attrs)*
        impl #default_generics #default_ty #default_where {
            /// Serves the actor through a channel the caller owns, in place of
            /// the default unbounded one.
            ///
            /// A bounded channel is how backpressure is asked for.
            pub fn channel<__ActifyTx, __ActifyRx>(
                self,
                __actify_channel: (__ActifyTx, __ActifyRx),
            ) -> #builder<#(#names,)* #view, (__ActifyTx, __ActifyRx)>
            where
                __ActifyTx: #root::JobSender<#call_ty>,
                __ActifyRx: #root::JobReceiver<#call_ty>,
            {
                #builder(self.0.channel(__actify_channel))
            }

            #[doc = #build_doc]
            pub fn build(
                self,
            ) -> (
                #handle<#(#names,)* #view, #root::DefaultSender<#call_ty>>,
                #actor_type,
            ) {
                let (__actify_handle, __actify_actor) = self.0.build(#run);
                (#handle(__actify_handle), __actify_actor)
            }
        }

        #(#attrs)*
        impl #own_generics #own_ty #own_where {
            #[doc = #build_doc]
            pub fn build(
                self,
            ) -> (
                #handle<#(#names,)* #view, __ActifyTx>,
                #actor_type,
            ) {
                let (__actify_handle, __actify_actor) = self.0.build(#run);
                (#handle(__actify_handle), __actify_actor)
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
                /// [`Park`](::actify::blocking::Wait::Park) by default;
                /// [`Spin`](::actify::blocking::Wait::Spin) busy-waits.
                pub fn wait(self, __actify_wait: ::actify::blocking::Wait) -> Self {
                    Self(self.0.wait(__actify_wait))
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
    let thunk = (carrier(method) == Carrier::Thunk).then(|| {
        let prefix = super::handle::build_call_prefix(info);
        quote! {
            __actify_thunk: |__actify_inner #(, #arg_names)*| {
                #prefix::#ident(__actify_inner #(, #arg_names)*)
            },
        }
    });

    quote! {
        #(#attrs)*
        pub #asyncness fn #ident #method_generics(&self, #(#arg_names: #arg_types),*) #return_type
        #where_clause
        {
            let (__actify_reply, __actify_rx) = #root::__private::reply();
            let __actify_call: #call_ty = #call::#variant {
                #(#arg_names,)*
                #thunk
                __actify_reply,
            };
            self.0.__call(__actify_call, __actify_rx) #awaiter
        }
    }
}
