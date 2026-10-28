---
applyTo: "**/*.rs"
---

# Rust documentation contracts

Document public and private Rust components with adjacent contracts.

## General Principles

Doc comments should be formatted with line comments (`///` or `//!`) in accordance with the [Rust Style Guide](https://doc.rust-lang.org/stable/style-guide/#doc-comments).

All public and private components **must** have documentation following the conventions defined in [The RustDoc Book](https://doc.rust-lang.org/rustdoc/how-to-write-documentation.html).

## Structure

### Summary Sentence
- **Always** start with a single-line summary that describes **what** the item does
- Use present tense ("Returns", "Creates", "Calculates" not "Will return", "Will create")
- Be concise and direct
- End with a period

### Additional Sections (when needed)
These sections are only needed if they are not implied by the summary sentence.
- Add blank line after summary before additional details
- Use `# Arguments` for parameter descriptions
- Use `# Returns` for complex return value explanations
- Use `# Errors` to document error conditions
- State non-obvious preconditions as `- Precondition:` bullets
- State postconditions not implied by the summary as `- Postcondition:` bullets
- Document non-O(1) time and space as a `- Complexity:` bullet
- Use `# Safety` for unsafe functions

## Specific Guidelines

### Functions and Methods
- Start with what the function **does** (not what it "will do")
- Check preconditions with `debug_assert!()`; do not document or test the consequences of violating them
- Distinguish runtime errors, documented in `# Errors`, from precondition violations
- Follow recommendations from [Better Code: Contracts](https://github.com/stlab/better-code/blob/main/better-code/src/chapter-2-contracts.md) adapted to Rust conventions
- For grammar production parsers, the production itself is sufficient documentation

**Good:**
```rust
/// `additive_expression = multiplicative_expression { ("+" | "-") multiplicative_expression }.`
fn is_additive_expression(&mut self) -> Result<bool>
```

**Bad:**
```rust
/// This function will parse additive expressions
fn is_additive_expression(&mut self) -> Result<bool>
```

### Structs and Enums
- Describe the purpose and role of the type
- State any invariants

**Good:**
```rust
/// A scope-based operation lookup with stack support.
///
/// Provides a stack of scopes for operation resolution, with built-in operations
/// as the fallback. Scopes are searched in LIFO order (most recently pushed first).
pub struct OpLookup { /* ... */ }
```

### Traits
- Describe what types implementing this trait represent
- Document trait semantics and contracts
- Provide examples of implementation

### Modules
- Use `//!` for module-level documentation
- Provide comprehensive overview with context
- Include examples demonstrating typical usage patterns
- Serve as a tutorial for the module's components

**Good:**
```rust
//! Operation table for dynamically dispatching operations based on type signatures.
//!
//! This module provides a scope-based registry for operations that can be looked up
//! based on an operation name (string) and the types of the operands.
//!
//! # Examples
//! ```
//! // Show typical usage
//! ```
```

### Type Aliases
- Explain what the alias represents
- Clarify why the alias exists (readability, semantics)

### Constants
- Describe what the constant represents
- Include units or context if applicable

## Error Documentation

When functions can return errors:
- List specific error conditions in `# Errors` section
- Be explicit about **when** errors occur

**Good:**
```rust
/// Looks up and applies an operation to the segment.
///
/// # Errors
///
/// Returns an error if no scope or built-in operation can handle the request.
pub fn lookup(&self, name: &str, types: &[TypeId], segment: &mut DynSegment) -> Result<()>
```

## Preconditions

State valid inputs rather than describing what happens when callers violate the
contract. Preconditions have unspecified behavior on violation; `# Safety` is
the exception that must describe undefined behavior for unsafe contracts.

**Good:**
```rust
/// Returns the type identifier at the given signature index.
///
/// - Precondition: `type_id_index` is within this signature.
///
fn type_id(&self, type_id_index: usize) -> TypeId
```

## Examples

Include `# Examples` for every public API. Add examples for non-obvious private
usage, initialization requirements, or multiple valid usage patterns when useful.

Use triple backticks with `rust` for proper syntax highlighting.

## Conciseness

- Avoid redundant information (don't restate the obvious from signatures)
- Focus on **why** and **when**, not just **what**
- Keep it brief but complete

## Cross-References

- Use backticks for code elements: `TypeId`, `DynSegment`, `OpLookup`
- Use `[Type]` for links to other documented items
- Link to related functions/types when helpful

## Special Cases

- **Getters/Setters:** Use a brief contract summary and include examples for public APIs
- **Grammar Productions:** The production itself is sufficient for parser functions
- **Internal/Private Items:** Contracts are required, including valid inputs, observable results, and non-constant complexity where applicable
