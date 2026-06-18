use cel_rs_macros::expression;

fn main() {
    expression! {
        undefined_var + 10
    };
}
