//! Playground tests for macro token output.

#[cfg(all(test, feature = "playground"))]

mod playground {
    #[test]
    fn tuple_index() {
        use cel_rs_macros::print_tokens;

        print_tokens! {(0,(0,1)).1.0.2.5}
    }
}
