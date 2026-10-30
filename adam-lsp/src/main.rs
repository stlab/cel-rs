//! `adam-lsp` binary entry point.

/// Runs the stdio server and propagates transport or lifecycle failures.
///
/// - Complexity: cumulative server dispatch/analysis time and current snapshot space.
fn main() -> anyhow::Result<()> {
    adam_lsp::run()
}
