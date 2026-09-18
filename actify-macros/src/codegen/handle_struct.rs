//! The handle an actor gets of its own.
//!
//! Where a trait on the library's generic handle could only ever send what that
//! handle already carried, a struct of the actor's own fixes its message type
//! to the generated enum. That is what lets a call travel as data.

use crate::parse::{ImplInfo, MethodInfo};
use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::Ident;

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

    let declared = declared_params(info);
    let names = param_names(info);
    let handle_ty = quote! { #handle<#(#names,)* #view, #sender> };
    let inner = quote! { ::actify::Handle<#impl_type, #view, #call_ty, #sender> };

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
        .push(syn::parse_quote!(#sender: ::actify::JobSender<#call_ty>));
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
    let made_ty = quote! { #handle<#(#names,)* #view, ::actify::DefaultSender<#call_ty>> };

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
        pub struct #handle <#(#declared,)* #view = #impl_type, #sender = ::actify::DefaultSender<#call_ty>>(
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
            pub async fn get(&self) -> #view {
                self.0.get().await
            }

            /// Overwrites the actor's value.
            ///
            /// # Panics
            ///
            /// Panics if the actor has stopped.
            pub async fn set(&self, __actify_val: #impl_type) {
                self.0.set(__actify_val).await
            }

            /// Returns a read-only handle to the same actor.
            pub fn read_handle(
                &self,
            ) -> ::actify::ReadHandle<#impl_type, #view, #call_ty, #sender> {
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

    let run = run_ident(info);

    // `Handle::new` exists only where actify can spawn, so this does too:
    // actify turns the macro's `tokio` feature on with its own.
    let new = cfg!(feature = "tokio").then(|| {
        quote! {
            /// Builds the actor, spawns it on Tokio and returns its handle.
            ///
            /// The spelling of [`builder`](Self::builder) followed by a
            /// `tokio::spawn`. Turn actify's `tokio` feature off and the caller
            /// spawns the actor future instead.
            #[track_caller]
            pub fn new(__actify_val: #impl_type) -> Self {
                #handle(::actify::__private::spawn(__actify_val, #run))
            }
        }
    });

    let builder = builder_ident(info);
    let names = param_names(info);
    quote! {
        #new

        /// Starts building a handle whose actor future the caller spawns.
        ///
        /// `build` hands back this handle and that future, so the caller
        /// chooses the executor.
        #[track_caller]
        pub fn builder(__actify_val: #impl_type) -> #builder<#(#names,)* #view> {
            #builder(::actify::__private::builder(__actify_val))
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
    let default_ty = quote! { #builder<#(#names,)* #view, ::actify::DefaultChannel> };

    let mut own_channel = common.clone();
    own_channel
        .params
        .push(syn::parse_quote!(__ActifyTx: ::actify::JobSender<#call_ty>));
    own_channel
        .params
        .push(syn::parse_quote!(__ActifyRx: ::actify::JobReceiver<#call_ty>));
    let (own_generics, _, own_where) = own_channel.split_for_impl();
    let own_ty = quote! { #builder<#(#names,)* #view, (__ActifyTx, __ActifyRx)> };

    let mut bare = info.generics.clone();
    bare.params.push(syn::parse_quote!(#view));
    bare.params.push(syn::parse_quote!(#channel));
    let (bare_generics, _, _) = bare.split_for_impl();

    let doc = format!(
        "Builds a handle to a `{}` actor, and the future that serves it.",
        quote! { #impl_type },
    );
    let debug_name = builder.to_string();

    quote! {
        #[doc = #doc]
        #(#attrs)*
        pub struct #builder<
            #(#declared,)*
            #view = #impl_type,
            #channel = ::actify::DefaultChannel,
        >(::actify::HandleBuilder<#impl_type, #view, #call_ty, #channel>);

        #(#attrs)*
        impl #bare_generics ::std::fmt::Debug for #builder_ty {
            fn fmt(&self, __actify_f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                ::std::write!(__actify_f, "{}", #debug_name)
            }
        }

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
                __ActifyTx: ::actify::JobSender<#call_ty>,
                __ActifyRx: ::actify::JobReceiver<#call_ty>,
            {
                #builder(self.0.channel(__actify_channel))
            }

            /// Returns the handle and the future that serves it.
            ///
            /// Nothing runs until the caller polls that future.
            pub fn build(
                self,
            ) -> (
                #handle<#(#names,)* #view, ::actify::DefaultSender<#call_ty>>,
                impl ::std::future::Future<Output = ()> + Send,
            ) {
                let (__actify_handle, __actify_actor) = self.0.build(#run);
                (#handle(__actify_handle), __actify_actor)
            }
        }

        #(#attrs)*
        impl #own_generics #own_ty #own_where {
            /// Returns the handle and the future that serves it, over the
            /// channel given to `channel`.
            pub fn build(
                self,
            ) -> (
                #handle<#(#names,)* #view, __ActifyTx>,
                impl ::std::future::Future<Output = ()> + Send,
            ) {
                let (__actify_handle, __actify_actor) = self.0.build(#run);
                (#handle(__actify_handle), __actify_actor)
            }
        }
    }
}

/// One method on the generated handle.
fn method(method: &MethodInfo, info: &ImplInfo, call_ty: &TokenStream) -> TokenStream {
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
        pub async fn #ident #method_generics(&self, #(#arg_names: #arg_types),*) #return_type
        #where_clause
        {
            let (__actify_reply, __actify_rx) = ::actify::__private::reply();
            let __actify_call: #call_ty = #call::#variant {
                #(#arg_names,)*
                #thunk
                __actify_reply,
            };
            self.0.__call(__actify_call, __actify_rx).await
        }
    }
}
