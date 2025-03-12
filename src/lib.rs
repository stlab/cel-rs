/*!
cel-rs provides a Forth-like runtime for developing domain specific languages. A program is composed of segments, where each segment is a sequence of operations.

Segments can be created in two ways.

1. Using the `DynSegment` struct which validates the type safety of the operations at runtime as the segment is built.
2. Using the `Segment` struct, which validates the type safety of the operations at compile time.

The two types of segments can be converted to each other [not yet implemented].

# Examples

```rust
use cel_rs::type_list::{List, IntoList};

// Create a type list from a tuple
let list = (1, "hello", 3.14).into_list();
```
*/
pub mod dyn_segment;
pub mod raw_segment;
pub mod raw_sequence;
pub mod raw_stack;
pub mod segment;
pub mod type_list;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parser() -> anyhow::Result<()> {
        use dyn_segment::DynSegment;
        use std::str::FromStr;

        // Simple tokenizer that handles quoted strings
        fn tokenize(input: &str) -> Result<Vec<String>, anyhow::Error> {
            let mut tokens = Vec::new();
            let mut chars = input.chars().peekable();
            let mut current = String::new();

            while let Some(&c) = chars.peek() {
                match c {
                    '"' => {
                        if !current.is_empty() {
                            tokens.push(current.clone());
                            current.clear();
                        }
                        chars.next(); // consume opening quote
                        let mut quoted = String::new();
                        let mut found_closing = false;
                        while let Some(&c) = chars.peek() {
                            if c == '"' {
                                chars.next();
                                found_closing = true;
                                break;
                            }
                            quoted.push(chars.next().unwrap());
                        }
                        if !found_closing {
                            return Err(anyhow::anyhow!("Unterminated string literal"));
                        }
                        tokens.push(format!("\"{}\"", quoted));
                    }
                    c if c.is_whitespace() => {
                        chars.next();
                        if !current.is_empty() {
                            tokens.push(current.clone());
                            current.clear();
                        }
                    }
                    '+' | '*' | '(' | ')' => {
                        chars.next();
                        if !current.is_empty() {
                            tokens.push(current.clone());
                            current.clear();
                        }
                        tokens.push(c.to_string());
                    }
                    _ => {
                        current.push(chars.next().unwrap());
                    }
                }
            }
            if !current.is_empty() {
                tokens.push(current);
            }
            Ok(tokens)
        }

        // Parse a value and add operations to segment
        fn parse_value(
            tokens: &[String],
            pos: &mut usize,
            seg: &mut DynSegment,
        ) -> anyhow::Result<()> {
            if *pos >= tokens.len() {
                return Err(anyhow::anyhow!("Unexpected end of input"));
            }

            match &tokens[*pos] as &str {
                // Try parsing as integer first
                tok if i32::from_str(tok).is_ok() => {
                    let val = i32::from_str(tok).unwrap();
                    seg.op0(move || val);
                    *pos += 1;
                }
                // Handle quoted strings
                tok if tok.starts_with('"') && tok.ends_with('"') => {
                    let val = tok[1..tok.len() - 1].to_string();
                    seg.op0(move || val.clone());
                    *pos += 1;
                }
                // Handle to_string()
                "to_string" => {
                    *pos += 1;
                    if *pos >= tokens.len() || tokens[*pos] != "(" {
                        return Err(anyhow::anyhow!("Expected ( after to_string"));
                    }
                    *pos += 1;
                    parse_value(tokens, pos, seg)?;
                    if *pos >= tokens.len() || tokens[*pos] != ")" {
                        return Err(anyhow::anyhow!("Expected ) after value"));
                    }
                    *pos += 1;
                    seg.op1(|x: i32| x.to_string())?;
                }
                _ => return Err(anyhow::anyhow!("Invalid value token: {}", tokens[*pos])),
            }
            Ok(())
        }

        // Parse multiplication: value (* value)*
        fn parse_term(
            tokens: &[String],
            pos: &mut usize,
            seg: &mut DynSegment,
        ) -> anyhow::Result<()> {
            parse_value(tokens, pos, seg)?;

            while *pos < tokens.len() && tokens[*pos] == "*" {
                *pos += 1;
                parse_value(tokens, pos, seg)?;
                seg.op2(|a: i32, b: i32| a * b)?;
            }
            Ok(())
        }

        // Parse addition/concatenation: term (+ term)*
        fn parse_expr(
            tokens: &[String],
            pos: &mut usize,
            seg: &mut DynSegment,
        ) -> anyhow::Result<()> {
            parse_term(tokens, pos, seg)?;

            while *pos < tokens.len() && tokens[*pos] == "+" {
                *pos += 1;
                parse_term(tokens, pos, seg)?;

                // Try string concatenation first
                if let Ok(()) = seg.op2(|a: String, b: String| a + &b) {
                    continue;
                }
                // Fall back to integer addition
                seg.op2(|a: i32, b: i32| a + b)?;
            }

            // Verify we've consumed all tokens
            if *pos < tokens.len() {
                return Err(anyhow::anyhow!("Unexpected token: {}", tokens[*pos]));
            }
            Ok(())
        }

        // Update test cases to handle Result from tokenize
        let mut seg1 = DynSegment::new::<()>();
        let tokens1 = tokenize("2 + 3 * 4 + 5")?;
        let mut pos = 0;
        parse_expr(&tokens1, &mut pos, &mut seg1)?;
        assert_eq!(seg1.call0::<i32>()?, 19);

        let mut seg2 = DynSegment::new::<()>();
        let tokens2 = tokenize("\"hello\" + \"world\"")?;
        let mut pos = 0;
        parse_expr(&tokens2, &mut pos, &mut seg2)?;
        assert_eq!(seg2.call0::<String>()?, "helloworld");

        let mut seg3 = DynSegment::new::<()>();
        let tokens3 = tokenize("to_string ( 42 ) + \"hello\"")?;
        let mut pos = 0;
        parse_expr(&tokens3, &mut pos, &mut seg3)?;
        assert_eq!(seg3.call0::<String>()?, "42hello");

        // Test error on unmatched quotes
        assert!(tokenize("\"hello + \"world\"").is_err());

        Ok(())
    }
}
