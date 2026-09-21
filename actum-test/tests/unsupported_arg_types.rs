/// Tests that the actum macro rejects invalid inputs with clear error messages.
///
/// If a compile_error! is accidentally removed, these tests will fail because the test file
/// will suddenly compile when it shouldn't.
#[test]
fn compile_fail_tests() {
    // The .stderr files match the exact diagnostics of one rustc version, so
    // every CI job must choose: the trybuild job, pinned to that version, sets
    // TRYBUILD_TESTS=1 and runs these; the test matrix on unpinned stable sets
    // TRYBUILD_TESTS=0. An unset variable on CI fails rather than skips, so the
    // suite cannot go dark through a lost job or variable. Locally the tests
    // always run. See CONTRIBUTING.md for how to regenerate the .stderr files.
    if std::env::var_os("CI").is_some() {
        match std::env::var("TRYBUILD_TESTS").as_deref() {
            Ok("1") => {}
            Ok("0") => {
                eprintln!("skipping trybuild tests: TRYBUILD_TESTS=0");
                return;
            }
            _ => panic!("CI is set but TRYBUILD_TESTS is not: set 1 to run or 0 to skip"),
        }
    }

    let t = trybuild::TestCases::new();

    // Argument types. The reference, raw-pointer and impl-Trait arms are
    // asserted by multiple_errors below, which hits all three in one block.
    t.compile_fail("tests/compile_fail/unsupported_arg_type.rs");

    // Return type validation
    t.compile_fail("tests/compile_fail/reference_return.rs");
    t.compile_fail("tests/compile_fail/impl_trait_return.rs");

    // Method validation
    t.compile_fail("tests/compile_fail/static_method.rs");
    t.compile_fail("tests/compile_fail/unsafe_method.rs");

    // Method generics, which a variant of the message enum cannot hold. One
    // check covers type and const parameters alike, so one case does too.
    t.compile_fail("tests/compile_fail/method_generic.rs");
    t.compile_fail("tests/compile_fail/async_method_where_clause.rs");
    t.compile_fail("tests/compile_fail/async_method_in_blocking_block.rs");
    t.compile_fail("tests/compile_fail/by_value_self.rs");

    // A handle sends through &mut, so one handle is one call at a time
    t.compile_fail("tests/compile_fail/two_calls_on_one_handle.rs");

    // Names the generated handle already uses
    t.compile_fail("tests/compile_fail/reserved_method_name.rs");

    // Skipped methods
    t.compile_fail("tests/compile_fail/skipped_method_not_on_handle.rs");

    // Invalid custom name
    t.compile_fail("tests/compile_fail/invalid_custom_name.rs");

    // Error reporting quality
    t.compile_fail("tests/compile_fail/multiple_errors.rs");
    t.compile_fail("tests/compile_fail/error_does_not_cascade.rs");
}
