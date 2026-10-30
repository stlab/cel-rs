use cel_rs_macros::expression;
/// Requires a compiler diagnostic for an expression missing its right operand.
fn main() {
    expression! {
        10 +
    };
}
