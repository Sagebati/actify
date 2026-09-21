# Contributing

This is a hard fork of [actify](https://github.com/AvalorAI/actify). It is
not published to crates.io and does not track upstream; changes here are
changes to this repository alone.

## Building and testing

The MSRV is 1.85. CI runs the following, and all of it must pass locally before
pushing:

```sh
cargo test --workspace
cargo test -p actify                                                  # default features
cargo check -p actify --no-default-features                           # no runtime
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all --check
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features
```

Dev-dependencies add tokio features that the library's own manifest does not
declare, so `cargo check -p actify` is the only run that proves the declared
features compile the library.

Documentation is checked twice because feature-gated items cannot be linked from
text that is always compiled.

The MSRV is declared in three places that must agree: `rust-version` in
`[workspace.package]`, the `MSRV` variable in `ci.yml`, and the line above.

## Visibility

There is no `pub(crate)` or `pub(super)` in the crate, and none should be
added. An item is private to its module and reached by that module's
children, or it is `pub` and reachable from the crate root. The two
`__private` modules exist for generated code alone: they are the items the
macro's output names, and nothing in the crate's own modules goes through
them.

## What the macro generates

Read the expansion before and after changing the macro:

```sh
cargo expand --example no_runtime_at_all
```

Every `#[allow]` in generated code covers a lint that a deny-warnings build
of a real actor was shown to trigger without it, and the reason is in a
comment beside it. Do not add one for a lint that has not fired.

## Pinned actions

Every action in `ci.yml` is pinned to a commit SHA, with the ref it came from
in a trailing comment. A tag can be moved to a different commit, so a tag is
not a pin.

To move one, resolve the ref and replace both the SHA and the comment:

```sh
gh api repos/actions/checkout/git/ref/tags/v5 --jq '.object.sha'
gh api repos/dtolnay/rust-toolchain/branches/master --jq '.commit.sha'
```

`dtolnay/rust-toolchain` takes its default toolchain from the branch the ref
points at, so pinning by SHA would hide which toolchain a job installs. The two
jobs that do not want plain stable use the `master` SHA and pass `toolchain`
explicitly, from `MSRV` and `TRYBUILD_TOOLCHAIN`.

## Compile-fail snapshots

`actify-test/tests/compile_fail/` holds trybuild cases with committed `.stderr`
files. They are skipped unless `TRYBUILD_TESTS` is set, because they assert exact
rustc diagnostics and only match the toolchain in `TRYBUILD_TOOLCHAIN`
(`.github/workflows/ci.yml`).

Run them, and regenerate the snapshots after changing a macro diagnostic:

```sh
TRYBUILD_TESTS=1 cargo test -p actify-test --test unsupported_arg_types
TRYBUILD=overwrite TRYBUILD_TESTS=1 cargo test -p actify-test --test unsupported_arg_types
```

Regenerate with the pinned toolchain, otherwise the committed output will not
match what CI produces. A snapshot that quotes a rustc diagnostic rather than
one of the macro's own `compile_error!` messages can only match one toolchain
at a time; `skipped_method_not_on_handle.stderr` and
`two_calls_on_one_handle.stderr` are such cases.

A case earns its place by being the only thing that fails when one validator
branch breaks. Two cases asserting the same message on the same branch are
one case too many.

## Allocation tests

`actify-test/tests/allocations.rs` and `blocking_allocations.rs` count what a
single call allocates and assert an exact number. Each lives in its own
binary because the counter is process-wide, and each holds a single test for
the same reason. A change that moves the count is a change to the crate's
contract and belongs in the changelog.
