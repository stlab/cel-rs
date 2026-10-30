use cel_rs_macros::expression;
/// Requires a compiler diagnostic for an unbound CEL identifier.
fn main() {
    expression! {
        undefined_var + 10
    };
}
