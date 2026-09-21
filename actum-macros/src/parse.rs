use proc_macro2::Span;
use syn::{
    Attribute, Error, FnArg, Generics, Ident, ImplItem, ImplItemFn, ItemImpl, Path, Receiver,
    ReturnType, Type, punctuated::Punctuated, spanned::Spanned, token::Comma,
};

/// Merge an error into an accumulator so that one pass over the impl block can
/// report every problem it finds, instead of one per recompile.
fn accumulate(errors: &mut Option<Error>, error: Error) {
    match errors {
        Some(existing) => existing.combine(error),
        None => *errors = Some(error),
    }
}

/// Which of the two actors an impl block asked for.
///
/// The difference is narrow enough that one codegen path emits both: a handful
/// of tokens, which `codegen::backend` lists, plus the rule below that a
/// blocking actor has no runtime to drive an `async fn` with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// `#[actum]`: the actor is a future, spawned on an executor.
    Async,
    /// `#[actum(blocking)]`: the actor is a loop, running on a thread.
    Blocking,
}

/// Intermediate representation for an entire impl block processed by `#[actum]`.
pub struct ImplInfo {
    /// Whether this block asked for a blocking actor.
    pub backend: Backend,
    /// The full impl type, e.g. `TestStruct<T>`.
    pub impl_type: Box<Type>,
    /// Generated handle name, e.g. `TestStructHandle`.
    pub handle_trait_ident: Ident,
    /// Generated message enum name, e.g. `TestStructCall`.
    pub call_enum_ident: Ident,
    /// Impl-level generics (where clause guaranteed present via `make_where_clause`).
    pub generics: Generics,
    /// If this is a trait impl, the trait path (e.g. `ActorVec<T>`).
    pub trait_path: Option<Path>,
    /// Filtered cfg/doc attributes from the impl block.
    pub attributes: Vec<Attribute>,
    /// Parsed methods.
    pub methods: Vec<MethodInfo>,
    /// The original (mutated) impl block, included in output for passthrough.
    pub original_impl: ItemImpl,
}

impl ImplInfo {
    /// Parse an `ItemImpl` into the intermediate representation.
    /// Mutates the impl block to ensure a where clause exists.
    pub fn from_impl_block(
        impl_block: &mut ItemImpl,
        custom_name: Option<syn::LitStr>,
        backend: Backend,
    ) -> syn::Result<ImplInfo> {
        let type_ident = get_impl_type_ident(&impl_block.self_ty)?;

        // Ensure the where clause always exists so we can unwrap safely
        impl_block.generics.make_where_clause();

        // A custom name replaces the type's own in both generated names, so
        // that the handle and the message it sends stay a matching pair.
        let stem = match custom_name {
            Some(lit) => {
                let name = lit.value();
                syn::parse_str::<Ident>(&name).map_err(|_| {
                    Error::new_spanned(
                        &lit,
                        "invalid `name` value: must be a valid Rust identifier",
                    )
                })?
            }
            None => Ident::new(&format!("{type_ident}Handle"), Span::call_site()),
        };
        let handle_trait_ident = stem.clone();
        let call_enum_ident = Ident::new(
            &format!("{}Call", stem.to_string().trim_end_matches("Handle")),
            Span::call_site(),
        );

        let trait_path = impl_block.trait_.as_ref().map(|(_, path, _)| path.clone());

        let attributes = filter_attributes(&impl_block.attrs);

        // Parse every method before returning, so one pass reports all problems.
        let mut errors = None;
        let mut methods = Vec::new();
        for item in &impl_block.items {
            let ImplItem::Fn(method) = item else {
                continue;
            };

            // Skipped methods are not parsed at all, so a signature that no
            // actor call could express is allowed to stay in the block.
            if find_marker_attribute(&method.attrs, "skip").is_some() {
                continue;
            }

            match MethodInfo::from_impl_method(method, backend) {
                Ok(info) => methods.push(info),
                Err(error) => accumulate(&mut errors, error),
            }
        }

        if let Some(errors) = errors {
            return Err(errors);
        }

        Ok(ImplInfo {
            backend,
            impl_type: impl_block.self_ty.clone(),
            handle_trait_ident,
            call_enum_ident,
            generics: impl_block.generics.clone(),
            trait_path,
            attributes,
            methods,
            original_impl: impl_block.clone(),
        })
    }
}

/// Intermediate representation for a single method within the impl block.
pub struct MethodInfo {
    /// Original method name.
    pub ident: Ident,
    /// Whether the method takes `&mut self`.
    pub is_mutable: bool,
    /// Whether the method is async.
    pub is_async: bool,
    /// Argument identifiers. For destructuring patterns a positional name is generated.
    pub arg_names: Punctuated<Ident, Comma>,
    /// Argument types.
    pub arg_types: Punctuated<Type, Comma>,
    /// Return type (defaults to `()`).
    pub output_type: Box<Type>,
    /// Method-level generics (including where clause).
    pub method_generics: Generics,
    /// Filtered cfg/doc attributes.
    pub attributes: Vec<Attribute>,
}

impl MethodInfo {
    /// Parse a single `ImplItemFn` into its intermediate representation.
    fn from_impl_method(method: &ImplItemFn, backend: Backend) -> syn::Result<MethodInfo> {
        let ident = method.sig.ident.clone();

        let is_mutable = method.sig.inputs.iter().any(|arg| {
            matches!(
                arg,
                FnArg::Receiver(Receiver {
                    mutability: Some(_),
                    ..
                })
            )
        });
        let is_async = method.sig.asyncness.is_some();

        let mut errors = None;

        if let Err(error) = validate_method_name(method) {
            accumulate(&mut errors, error);
        }

        if let Err(error) = validate_signature_modifiers(method) {
            accumulate(&mut errors, error);
        }

        if let Err(error) = validate_method_generics(method) {
            accumulate(&mut errors, error);
        }

        if let Err(error) = validate_asyncness(method, backend) {
            accumulate(&mut errors, error);
        }

        if let Err(error) = validate_receiver(method) {
            accumulate(&mut errors, error);
        }

        let output_type = match &method.sig.output {
            ReturnType::Type(_, ty) => ty.clone(),
            ReturnType::Default => Box::new(syn::parse_quote! { () }),
        };

        if let Err(error) = validate_return_type(&output_type) {
            accumulate(&mut errors, error);
        }

        let (arg_names, arg_types) = match transform_args(&method.sig.inputs) {
            Ok(args) => args,
            Err(error) => {
                accumulate(&mut errors, error);
                (Punctuated::new(), Punctuated::new())
            }
        };

        if let Some(errors) = errors {
            return Err(errors);
        }

        let attributes = filter_attributes(&method.attrs);

        Ok(MethodInfo {
            ident,
            is_mutable,
            is_async,
            arg_names,
            arg_types,
            output_type,
            method_generics: method.sig.generics.clone(),
            attributes,
        })
    }
}

/// Extract the type name from a named type path (e.g. `MyStruct` from `MyStruct<T>`).
/// Returns the last path segment's ident, so `crate::module::Foo<T>` yields `Foo`.
fn get_impl_type_ident(impl_type: &Type) -> syn::Result<Ident> {
    let last_segment = match impl_type {
        Type::Path(type_path) => type_path.path.segments.last(),
        _ => None,
    };

    last_segment
        .map(|segment| segment.ident.clone())
        .ok_or_else(|| {
            Error::new_spanned(
                impl_type,
                "The actum macro requires a named type path (e.g. `impl MyStruct`), not a reference, tuple, or other type expression",
            )
        })
}

/// Built-in compiler attributes that are safe to propagate onto the generated
/// handle's methods and its message enum's variants. Everything else (proc-macro attributes
/// like `#[instrument]`, actum-specific attributes like `#[skip]`)
/// is stripped so it only appears on the original impl method where it belongs.
const PROPAGATED_ATTRIBUTES: &[&str] = &[
    "doc",
    "allow",
    "warn",
    "deny",
    "forbid",
    "cfg",
    "cfg_attr",
    "deprecated",
    "must_use",
];

/// Returns `true` if the attribute is in the [`PROPAGATED_ATTRIBUTES`] whitelist.
///
/// Only single-segment paths are checked (all built-in compiler attributes are
/// single-segment). Multi-segment paths like `tracing::instrument` or
/// `actum::skip` are always excluded.
fn is_propagated_attribute(attr: &Attribute) -> bool {
    let segments = &attr.path().segments;
    segments.len() == 1
        && segments
            .first()
            .is_some_and(|seg| PROPAGATED_ATTRIBUTES.contains(&seg.ident.to_string().as_str()))
}

/// Keep only whitelisted built-in attributes for generated code.
///
/// A proc-macro attribute like `#[instrument]` rewrites the function it is
/// placed on, which is meant for the user's method and not for a generated
/// method that forwards a call to it. An actum attribute like `#[skip]` has
/// already done its work during parsing. Both are stripped and remain only on
/// the original impl method.
fn filter_attributes(attrs: &[Attribute]) -> Vec<Attribute> {
    attrs
        .iter()
        .filter(|attr| is_propagated_attribute(attr))
        .cloned()
        .collect()
}

/// Find actum's `skip` marker attribute.
///
/// Only the two spellings actum actually exports are recognised: bare
/// (`#[skip]`, via a `use`) and crate-qualified (`#[actum::skip]`). Matching a
/// name anywhere in the path would claim another crate's attribute.
fn find_marker_attribute<'a>(attrs: &'a [Attribute], name: &str) -> Option<&'a Attribute> {
    attrs.iter().find(|attr| {
        let path = attr.path();
        path.is_ident(name)
            || (path.segments.len() == 2
                && path.segments[0].ident == "actum"
                && path.segments[1].ident == name)
    })
}

/// Verify the method has a receiver and that it borrows rather than consumes.
fn validate_receiver(method: &ImplItemFn) -> syn::Result<()> {
    let receiver = method.sig.inputs.iter().find_map(|arg| match arg {
        FnArg::Receiver(receiver) => Some(receiver),
        FnArg::Typed(_) => None,
    });

    let Some(receiver) = receiver else {
        return Err(Error::new(
            method.span(),
            "Static method cannot be actified: the method requires a receiver to the impl type, using either &self or &mut self",
        ));
    };

    // The actor owns its state for as long as it runs, so it can only lend it
    // to a method. Consuming it would leave the actor without state to serve
    // the next job.
    if receiver.reference.is_none() {
        return Err(Error::new(
            receiver.span(),
            "Actor methods cannot take self by value: the actor owns its state for its entire lifetime, so use &self or &mut self",
        ));
    }

    Ok(())
}

/// Verify the method has no signature modifier the generated handle cannot honour.
fn validate_signature_modifiers(method: &ImplItemFn) -> syn::Result<()> {
    if let Some(unsafety) = method.sig.unsafety {
        return Err(Error::new(
            unsafety.span,
            "Unsafe methods cannot be actified: the generated handle would call them from safe code, so the safety contract could not be upheld",
        ));
    }

    Ok(())
}

/// Validate that a call on this method can travel as a message.
///
/// A call is a variant of the actor's message enum, which takes the impl
/// block's parameters and nothing else, so a method's own generics have
/// nowhere to live. A `where` clause is allowed instead: the variant carries a
/// pointer to the method, built where the bound is in scope. That trick needs
/// the method to return its result rather than a future, so an async method
/// cannot use it.
fn validate_method_generics(method: &ImplItemFn) -> syn::Result<()> {
    if let Some(param) = method.sig.generics.params.first() {
        return Err(Error::new_spanned(
            param,
            "Actor methods cannot declare generic parameters of their own: a call travels as a variant of the actor's message enum, which has no such parameter to hold the arguments (use a concrete type, e.g. fn(usize) -> usize rather than F: Fn(usize) -> usize)",
        ));
    }

    let bounded = method
        .sig
        .generics
        .where_clause
        .as_ref()
        .is_some_and(|clause| !clause.predicates.is_empty());

    if bounded && method.sig.asyncness.is_some() {
        return Err(Error::new_spanned(
            method.sig.generics.where_clause.as_ref().unwrap(),
            "An async actor method cannot carry a where clause of its own: the call would have to reach it through a function pointer, and one to an async method returns a future the message cannot hold",
        ));
    }

    Ok(())
}

/// The methods every generated handle has, whatever its actor declares.
///
/// An actor method of the same name would be a second definition of it on the
/// same type, which rustc reports as `E0592` pointing at the `#[actum]`
/// attribute rather than at the method, with nothing to say why.
const RESERVED_METHOD_NAMES: [&str; 2] = ["new", "builder"];

/// Validate that the method's name is still free on the generated handle.
///
/// `new` is only emitted where actum can spawn, but it is reserved either way:
/// a name that compiles with one feature set and not another is worse than one
/// that never compiles.
fn validate_method_name(method: &ImplItemFn) -> syn::Result<()> {
    let name = method.sig.ident.to_string();
    if RESERVED_METHOD_NAMES.contains(&name.as_str()) {
        return Err(Error::new(
            method.sig.ident.span(),
            format!(
                "`{name}` is one of the methods every generated handle already has, so the handle \
                 would define it twice (rename the method, or keep it off the handle with \
                 #[actum::skip] and call it on the actor directly)"
            ),
        ));
    }

    Ok(())
}

/// Validate that the backend can run a method of this kind.
///
/// A blocking actor is a loop on a thread with no executor anywhere near it,
/// so there is nothing to drive the future an `async fn` returns. Blocking on
/// it would mean picking a runtime for the caller, which is the dependency the
/// blocking backend exists to avoid.
fn validate_asyncness(method: &ImplItemFn, backend: Backend) -> syn::Result<()> {
    if backend == Backend::Blocking {
        if let Some(asyncness) = method.sig.asyncness {
            return Err(Error::new(
                asyncness.span,
                "An async method cannot be actified in a #[actum(blocking)] block: the actor runs on a thread with no executor to drive the future (make the method synchronous, or use #[actum] and spawn the actor on a runtime)",
            ));
        }
    }

    Ok(())
}

/// Validate that a return type can travel back from the actor task.
///
/// A result travels home through a reply channel the call carries, so it has
/// to be an owned `'static` type, and the generated code names it in a `let`
/// binding.
///
/// Unlike arguments this rejects a known set rather than accepting one: return
/// types are more varied, and an allowlist risks refusing something that
/// compiles fine today.
fn validate_return_type(ty: &Type) -> syn::Result<()> {
    match ty {
        // `&'static T` outlives the actor and boxes fine. Any other borrow is
        // tied to the actor's state and cannot leave the task with the result.
        Type::Reference(reference)
            if reference
                .lifetime
                .as_ref()
                .is_none_or(|lifetime| lifetime.ident != "static") =>
        {
            Err(Error::new_spanned(
                ty,
                "Actor methods must return owned types or 'static references: a result borrowed from the actor state cannot outlive the call (e.g. return String instead of &str)",
            ))
        }

        Type::ImplTrait(_) => Err(Error::new_spanned(
            ty,
            "impl Trait is not supported as an actor method return type; return a concrete owned type instead (e.g. Vec<T> rather than impl Iterator<Item = T>)",
        )),

        Type::Ptr(_) => Err(Error::new_spanned(
            ty,
            "Raw pointer types (*const T, *mut T) are not supported as actor method return types because they are not Send",
        )),

        _ => Ok(()),
    }
}

/// Extract and validate argument names and types from method inputs.
/// For ident patterns, uses the original name. For non-ident patterns (e.g.
/// destructuring `(a, b): (i32, i32)`), generates a positional name so the
/// handle can box/unbox the value; the original method destructures at the call site.
#[allow(clippy::type_complexity)]
fn transform_args(
    args: &Punctuated<FnArg, Comma>,
) -> syn::Result<(Punctuated<Ident, Comma>, Punctuated<Type, Comma>)> {
    let mut arg_names: Punctuated<Ident, Comma> = Punctuated::new();
    let mut arg_types: Punctuated<Type, Comma> = Punctuated::new();
    let mut errors = None;

    for (i, arg) in args.iter().enumerate() {
        match arg {
            syn::FnArg::Typed(pat_type) => {
                // Every argument is checked, so a method with several invalid
                // ones reports them all at once.
                if let Err(error) = validate_arg_type(&pat_type.ty, pat_type.ty.span()) {
                    accumulate(&mut errors, error);
                    continue;
                }

                let ident = match &*pat_type.pat {
                    syn::Pat::Ident(pat_ident) => pat_ident.ident.clone(),
                    _ => Ident::new(&format!("_arg{}", i), Span::call_site()),
                };

                arg_names.push(ident);
                arg_types.push(*pat_type.ty.clone());
            }
            // Checked by validate_receiver; it contributes no boxed argument.
            syn::FnArg::Receiver(_) => {}
        }
    }

    match errors {
        Some(errors) => Err(errors),
        None => Ok((arg_names, arg_types)),
    }
}

/// Validate that an argument type is supported for actor method arguments.
fn validate_arg_type(ty: &Type, span: proc_macro2::Span) -> syn::Result<()> {
    match ty {
        // Valid owned types
        Type::Path(_)
        | Type::Tuple(_)
        | Type::Array(_)
        | Type::BareFn(_)
        | Type::Paren(_)
        | Type::Group(_) => Ok(()),

        Type::Reference(_) => Err(Error::new(
            span,
            "Input arguments of actor model methods must be owned types, not references (e.g. use String instead of &str)",
        )),

        Type::Ptr(_) => Err(Error::new(
            span,
            "Raw pointer types (*const T, *mut T) are not supported as actor method arguments because they are not Send",
        )),

        Type::ImplTrait(_) => Err(Error::new(
            span,
            "impl Trait is not supported as an actor method argument; use a named generic type parameter with trait bounds instead (e.g. fn method<F: Fn()>(&self, f: F))",
        )),

        _ => Err(Error::new(
            span,
            "Unsupported argument type for actor method; use a concrete owned type (e.g. String, Vec<T>, (A, B), [T; N])",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    fn attr(tokens: Attribute) -> Attribute {
        tokens
    }

    #[test]
    fn whitelisted_attributes_are_propagated() {
        let attrs: Vec<Attribute> = vec![
            attr(parse_quote!(#[doc = "hello"])),
            attr(parse_quote!(#[allow(unused)])),
            attr(parse_quote!(#[warn(missing_docs)])),
            attr(parse_quote!(#[deny(warnings)])),
            attr(parse_quote!(#[forbid(unsafe_code)])),
            attr(parse_quote!(#[cfg(test)])),
            attr(parse_quote!(#[cfg_attr(test, ignore)])),
            attr(parse_quote!(#[deprecated])),
            attr(parse_quote!(#[must_use])),
        ];

        let filtered = filter_attributes(&attrs);
        assert_eq!(
            filtered.len(),
            attrs.len(),
            "all whitelisted attributes should pass through"
        );
    }

    #[test]
    fn instrument_is_stripped() {
        let attrs: Vec<Attribute> = vec![
            attr(parse_quote!(#[doc = "keep me"])),
            attr(parse_quote!(#[instrument(level = "debug", skip_all)])),
        ];

        let filtered = filter_attributes(&attrs);
        assert_eq!(filtered.len(), 1);
        assert!(filtered[0].path().is_ident("doc"));
    }

    #[test]
    fn qualified_instrument_is_stripped() {
        let attrs: Vec<Attribute> = vec![
            attr(parse_quote!(#[tracing::instrument(skip_all)])),
            attr(parse_quote!(#[cfg(feature = "tracing")])),
        ];

        let filtered = filter_attributes(&attrs);
        assert_eq!(filtered.len(), 1);
        assert!(filtered[0].path().is_ident("cfg"));
    }

    #[test]
    fn actum_attrs_are_stripped() {
        let attrs: Vec<Attribute> = vec![
            attr(parse_quote!(#[skip])),
            attr(parse_quote!(#[actum::skip])),
            attr(parse_quote!(#[doc = "visible"])),
        ];

        let filtered = filter_attributes(&attrs);
        assert_eq!(filtered.len(), 1);
        assert!(filtered[0].path().is_ident("doc"));
    }

    #[test]
    fn marker_attributes_are_recognised_bare_and_qualified() {
        let attrs: Vec<Attribute> = vec![attr(parse_quote!(#[skip]))];
        assert!(find_marker_attribute(&attrs, "skip").is_some());

        let attrs: Vec<Attribute> = vec![attr(parse_quote!(#[actum::skip]))];
        assert!(find_marker_attribute(&attrs, "skip").is_some());
    }

    #[test]
    fn marker_attributes_of_other_crates_are_ignored() {
        let attrs: Vec<Attribute> = vec![attr(parse_quote!(#[other_crate::skip]))];
        assert!(find_marker_attribute(&attrs, "skip").is_none());

        let attrs: Vec<Attribute> = vec![attr(parse_quote!(#[skip::configure]))];
        assert!(find_marker_attribute(&attrs, "skip").is_none());

        let attrs: Vec<Attribute> = vec![attr(parse_quote!(#[a::b::skip]))];
        assert!(find_marker_attribute(&attrs, "skip").is_none());
    }

    #[test]
    fn unknown_single_segment_attr_is_stripped() {
        let attrs: Vec<Attribute> = vec![
            attr(parse_quote!(#[serde(rename_all = "camelCase")])),
            attr(parse_quote!(#[deprecated])),
        ];

        let filtered = filter_attributes(&attrs);
        assert_eq!(filtered.len(), 1);
        assert!(filtered[0].path().is_ident("deprecated"));
    }
}
