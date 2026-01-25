# CEL Parser - Operation Lookup Example

This example demonstrates how the CEL parser uses a scope-based operation lookup system to dynamically dispatch operations based on type signatures.

## Basic Usage

```rust
use cel_parser::CELParser;
use proc_macro2::TokenStream;
use std::str::FromStr;

fn main() -> anyhow::Result<()> {
    // Parse and execute a simple arithmetic expression
    let input = TokenStream::from_str("10 + 20 * 3").unwrap();
    let mut parser = CELParser::new(input.into_iter());
    let mut segment = parser.parse()?;
    let result = segment.call0::<i32>()?;
    
    println!("Result: {}", result); // Output: 70 (due to operator precedence)
    
    Ok(())
}
```

## Custom Operations with Scopes

You can extend the operation lookup with custom scopes:

```rust
use cel_parser::CELParser;
use cel_runtime::DynSegment;
use proc_macro2::TokenStream;
use std::any::TypeId;
use std::str::FromStr;

fn main() -> anyhow::Result<()> {
    let input = TokenStream::from_str("10 + 20").unwrap();
    let mut parser = CELParser::new(input.into_iter());
    
    // Add a custom scope that overrides addition for i32
    parser.op_lookup_mut().push_scope(Box::new(|name, types, segment| {
        if name == "+" && types.len() == 2 && types[0] == TypeId::of::<i32>() {
            // Custom addition that adds an extra 100
            segment.op2(|a: i32, b: i32| a + b + 100)?;
            Ok(true) // Handled
        } else {
            Ok(false) // Not handled, try next scope
        }
    }));
    
    let mut segment = parser.parse()?;
    let result = segment.call0::<i32>()?;
    
    println!("Result: {}", result); // Output: 130 (10 + 20 + 100)
    
    Ok(())
}
```

## Adding Custom Identifiers

You can use scopes to provide values for identifiers:

```rust
use cel_parser::CELParser;
use cel_runtime::DynSegment;
use proc_macro2::TokenStream;
use std::any::TypeId;
use std::str::FromStr;

fn main() -> anyhow::Result<()> {
    let input = TokenStream::from_str("x + y").unwrap();
    let mut parser = CELParser::new(input.into_iter());
    
    // Add a scope that provides variable values
    parser.op_lookup_mut().push_scope(Box::new(|name, types, segment| {
        // 0-ary lookup means identifier
        if types.is_empty() {
            match name {
                "x" => {
                    segment.op0(|| 10i32);
                    Ok(true)
                }
                "y" => {
                    segment.op0(|| 20i32);
                    Ok(true)
                }
                _ => Ok(false)
            }
        } else {
            Ok(false)
        }
    }));
    
    let mut segment = parser.parse()?;
    let result = segment.call0::<i32>()?;
    
    println!("Result: {}", result); // Output: 30
    
    Ok(())
}
```

## How It Works

1. **Type Inspection**: When the parser encounters an operator (like `+`), it:
   - Gets the top N types from the DynSegment's stack using `peek_types_vec()`
   - Uses the operator symbol and TypeIds to look up the operation

2. **Scope-Based Dispatch**: The operation lookup system maintains a stack of scopes:
   - Scopes are searched in LIFO order (most recent first)
   - Built-in operations are checked last as a fallback
   - Each scope can handle, reject, or pass through to the next scope

3. **Operation Names**: Operations are identified by their operator symbols:
   - Arithmetic: `"+"`, `"-"`, `"*"`, `"/"`, `"%"`
   - Bitwise: `"&"`, `"|"`, `"^"`, `"<<"`, `">>"`
   - Logical: `"&&"`, `"||"`, `"!"`
   - Comparison: `"=="`, `"!="`, `"<"`, `"<="`, `">"`, `">="`
   - Identifiers and custom functions use their names

4. **Arity**: The number of operands distinguishes operations:
   - 0-ary: identifiers and constants
   - 1-ary: unary operators like `"-"` (negation) and `"!"` (logical not)
   - 2-ary: binary operators like `"+"`, `"*"`, etc.

## Standard Operations

The parser comes with built-in operations for:

- **Arithmetic**: `+`, `-`, `*`, `/`, `%` for integer and floating-point types
- **Bitwise**: `&`, `|`, `^`, `<<`, `>>` for integer types
- **Logical**: `&&`, `||`, `!` for boolean types
- **Comparison**: `==`, `!=`, `<`, `<=`, `>`, `>=` for all comparable types
- **Unary**: `-` (negation) for signed types, `!` (logical not) for booleans

## Type Support

Operations are registered for the following types:

- Unsigned integers: `u8`, `u16`, `u32`, `u64`, `u128`, `usize`
- Signed integers: `i8`, `i16`, `i32`, `i64`, `i128`, `isize`
- Floating point: `f32`, `f64`
- Boolean: `bool`

## Example with Different Types

```rust
use cel_parser::CELParser;
use proc_macro2::TokenStream;
use std::str::FromStr;

fn main() -> anyhow::Result<()> {
    // u32 addition
    let input = TokenStream::from_str("10u32 + 20u32").unwrap();
    let mut parser = CELParser::new(input.into_iter());
    let mut segment = parser.parse()?;
    let result = segment.call0::<u32>()?;
    println!("u32 result: {}", result); // Output: 30
    
    // f64 multiplication
    let input = TokenStream::from_str("3.5 * 2.0").unwrap();
    let mut parser = CELParser::new(input.into_iter());
    let mut segment = parser.parse()?;
    let result = segment.call0::<f64>()?;
    println!("f64 result: {}", result); // Output: 7.0
    
    // Boolean logic
    let input = TokenStream::from_str("true && false").unwrap();
    let mut parser = CELParser::new(input.into_iter());
    let mut segment = parser.parse()?;
    let result = segment.call0::<bool>()?;
    println!("bool result: {}", result); // Output: false
    
    Ok(())
}
```

## Scope Management

Scopes can be pushed and popped to create temporary overrides or extensions:

```rust
use cel_parser::CELParser;
use proc_macro2::TokenStream;
use std::str::FromStr;

fn main() -> anyhow::Result<()> {
    let mut parser = CELParser::new(TokenStream::new().into_iter());
    
    // Push a scope for a specific context
    parser.op_lookup_mut().push_scope(Box::new(|name, types, segment| {
        // Custom logic here
        Ok(false) // Pass through if not handled
    }));
    
    // Parse expressions with the scope active
    // ...
    
    // Pop the scope when done
    parser.op_lookup_mut().pop_scope();
    
    Ok(())
}
```

## Performance Characteristics

- **Built-in operations**: O(1) hash lookup by operator name, then O(k) linear scan through type signatures (typically k=1-15 per operator)
- **Type matching**: Single TypeId comparison for operations with matching operand types
- **Memory overhead**: Minimal - single TypeIds stored per signature, shared across all operations
- **Custom scopes**: O(n) where n is the number of scopes (typically very small)
