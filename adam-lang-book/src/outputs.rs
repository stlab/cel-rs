//! # Chapter 7: Outputs and Requirements
//!
//! ## 7.1 Grammar
//!
//! ```text
//! out_decl    = "out" identifier [ ":" type_expr ] ":=" expression
//!                [ "require" "{" { requirement } "}" ] ";".
//! requirement = identifier ":" expression ";".
//! ```
//!
//! An `out` declares a new cell computed by exactly one expression — there's no alternative
//! binding to choose between, unlike a `relationship`. Its dependencies are
//! [deduced](crate::expressions#34-deduced-dependencies) the same way. The `: type_expr`
//! annotation is optional; when absent, the output's type is inferred from the initializer, the
//! same rule [Chapter 2](crate::cells#23-built-in-types-and-inference) gives for a plain `cell`.
//!
//! ```rust
//! let mut parser = adam_lang_book::support::parser();
//! let mut parsed = parser
//!     .parse_str(
//!         r#"
//!         sheet area_demo {
//!             cell width: i32 = 10;
//!             cell height: i32 = 20;
//!
//!             out area := width * height;
//!         }
//!         "#,
//!     )
//!     .unwrap();
//! parsed.propagate().unwrap();
//! let area = parsed.cell_names["area"].0;
//! assert_eq!(*parsed.read::<i32>(area).unwrap(), 200);
//! ```
//!
//! ## 7.2 An output's cell is terminal
//!
//! `out` shares one namespace with `cell` — declaring `out result := ...;` after (or before) a
//! `cell result` in the same sheet is a duplicate-name error, exactly like two `cell`
//! declarations would be. Unlike a plain cell, though, an output's cell can never be *written*:
//! not by a host `write()` call, not by a `relationship` binding, not by a second `out`. It's
//! computed exactly once per `propagate()`, by its own initializer, and nothing else:
//!
//! ```rust
//! let mut parser = adam_lang_book::support::parser();
//! let mut parsed = parser
//!     .parse_str("sheet s { cell width: i32 = 10; out area := width * 2; }")
//!     .unwrap();
//! let output = parsed.output_names["area"];
//! let area_cell = parsed.output_cell(output).unwrap();
//! let err = parsed.write(area_cell, 999_i32).unwrap_err();
//! assert!(matches!(err, adam_rs::Error::TerminalCell));
//! ```
//!
//! An output cell is nonetheless an ordinary cell for *reading*: a later declaration in the same
//! sheet can reference an earlier `out` by name in its own expression, exactly like referencing
//! any other already-declared cell.
//!
//! ## 7.3 Requirements: diagnostics, not gates
//!
//! A `require { ... }` block trailing an `out`'s initializer names zero or more boolean checks.
//! Each `requirement`'s own dependencies are deduced separately from the output's initializer —
//! a requirement commonly reads the output's own value by name, alongside whatever other cells
//! it needs:
//!
//! ```rust
//! let mut parser = adam_lang_book::support::parser();
//! let mut parsed = parser
//!     .parse_str(
//!         r#"
//!         sheet area_demo {
//!             cell width: i32 = 10;
//!             cell height: i32 = 20;
//!
//!             out area: i32 := width * height require {
//!                 not_too_big: area <= 300;
//!             };
//!         }
//!         "#,
//!     )
//!     .unwrap();
//! parsed.propagate().unwrap();
//! let output = parsed.output_names["area"];
//! assert!(parsed.output_valid(output));
//!
//! let width = parsed.cell_names["width"].0;
//! parsed.write(width, 50_i32).unwrap();
//! parsed.propagate().unwrap();
//! assert!(!parsed.output_valid(output));
//! ```
//!
//! A failed requirement never stops `propagate()` from succeeding, and never stops `area` from
//! being computed and readable — `output_valid`/`violated_requirements` exist precisely because
//! nothing else in the sheet notices a requirement failing on its own. A requirement's `name` is
//! just a label passed through to the query API; it happens to read naturally when it echoes a
//! cell name (`not_too_big`, `width_max`), but it isn't a cell reference and doesn't have to
//! match one.
//!
//! ## 7.4 Multiple requirements
//!
//! An output can list any number of requirements; `violated_requirements` reports exactly the
//! ones currently failing, by [`adam_rs::RequirementId`]:
//!
//! ```rust
//! let mut parser = adam_lang_book::support::parser();
//! let mut parsed = parser
//!     .parse_str(
//!         r#"
//!         sheet bounds_demo {
//!             cell x: i32 = 50;
//!
//!             out clamped: i32 := x require {
//!                 not_negative: clamped >= 0;
//!                 not_too_big: clamped <= 100;
//!             };
//!         }
//!         "#,
//!     )
//!     .unwrap();
//! let output = parsed.output_names["clamped"];
//! let x = parsed.cell_names["x"].0;
//!
//! parsed.write(x, -10_i32).unwrap();
//! parsed.propagate().unwrap();
//! assert_eq!(parsed.violated_requirements(output).count(), 1);
//! assert!(!parsed.output_valid(output));
//! ```
