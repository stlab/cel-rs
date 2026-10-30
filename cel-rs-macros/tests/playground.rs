//! Playground tests for macro token output.

#[cfg(all(test, feature = "playground"))]
mod playground {
    /// Compiles token diagnostics over nested groups and tuple-like punctuation.
    #[test]
    fn token_output_accepts_nested_groups_and_punctuation() {
        use cel_rs_macros::print_tokens;

        print_tokens! {(0,(0,1)).1.0.2.5}
    }
}
