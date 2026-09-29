# actum-macros

The procedural macros behind this repository's `actum` crate, a hard fork of
[actify](https://github.com/AvalorAI/actify). This crate is not meant to be
used on its own: the code it generates reaches into `actum::__private`, so it
only compiles as part of an actum build, and it is pinned to the exact
version of `actum` it was built with.

Depend on `actum` from this repository instead, which re-exports what you
need. See its documentation (`cargo doc --open`) for what `#[actum]` and
`#[actum(blocking)]` generate and which method signatures they accept.
