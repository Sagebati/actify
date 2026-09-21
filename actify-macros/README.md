# actify-macros

The procedural macros behind this repository's `actify` crate, a hard fork of
[actify](https://github.com/AvalorAI/actify). This crate is not meant to be
used on its own: the code it generates reaches into `actify::__private`, so it
only compiles as part of an actify build, and it is pinned to the exact
version of `actify` it was built with.

Depend on `actify` from this repository instead, which re-exports what you
need. See its documentation (`cargo doc --open`) for what `#[actify]` and
`#[actify(blocking)]` generate and which method signatures they accept.
