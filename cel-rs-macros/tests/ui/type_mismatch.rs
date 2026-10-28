use cel_rs_macros::expression;
/// Requires a compiler diagnostic for incompatible addition operands.
fn main() {
    expression! {
        "Hello" + 32.0
    };
}
