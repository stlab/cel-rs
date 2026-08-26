//! # Chapter 5: Conditionals
//!
//! ## 5.1 Grammar
//!
//! ```text
//! conditional_decl   = "conditional" expression "{" { conditional_branch } "}".
//! conditional_branch = (expression | "_") "=>" "{" { relationship_decl } "}" [ "," ].
//! ```
//!
//! A `conditional` evaluates its **match subject** — the `expression` right after the
//! `conditional` keyword — and activates the one branch, if any, whose literal equals the
//! current match value. Only that branch's `relationship` blocks participate in the round's
//! solve; every other branch's relationships are invisible to the planner, exactly as if they
//! weren't declared. A branch's own body holds nothing but `relationship` declarations — a
//! `conditional` cannot declare cells, and cannot nest another `conditional` directly inside a
//! branch.
//!
//! ## 5.2 The match subject
//!
//! A bare cell name (`conditional mode { ... }`) is the common case — the match value is just
//! that cell's current value. The match subject can also be a general expression over several
//! already-declared cells, [deduced](crate::expressions#34-deduced-dependencies) exactly like a
//! relationship binding's body:
//!
//! ```rust
//! let mut parser = adam_lang_book::support::parser();
//! let mut parsed = parser
//!     .parse_str(
//!         r#"
//!         sheet resample_demo {
//!             cell resample: bool = true;
//!             cell constrain: bool = true;
//!             cell locked: bool = false;
//!
//!             conditional resample && constrain {
//!                 true => {
//!                     relationship { locked := true; }
//!                 }
//!                 false => {
//!                     relationship { locked := false; }
//!                 }
//!             }
//!         }
//!         "#,
//!     )
//!     .unwrap();
//! parsed.propagate().unwrap();
//! let locked = parsed.cell_names["locked"].0;
//! assert!(*parsed.read::<bool>(locked).unwrap());
//!
//! let resample = parsed.cell_names["resample"].0;
//! parsed.write(resample, false).unwrap();
//! parsed.propagate().unwrap();
//! assert!(!*parsed.read::<bool>(locked).unwrap());
//! ```
//!
//! Every branch literal's type must match the match subject's own inferred type exactly — a
//! sheet mixing an `i32` match subject with a `bool` branch literal fails to parse, not to
//! propagate. The match subject can also be tuple-valued, in which case each branch literal is a
//! tuple expression of the same shape.
//!
//! ## 5.3 The default branch
//!
//! `_ => { ... }` matches whatever value none of the named branches list, and — because it's
//! defined that way — must be the last branch textually; a named branch written after `_` is a
//! syntax error. Leaving off `_` entirely is legal: if the match value doesn't equal any
//! branch's literal, none of the conditional's relationships are active that round, and every
//! cell that would otherwise be one of their outputs stays free — reverting to its own last
//! externally-written value (or its declared default), not to whatever the branch last computed
//! for it. This is the same source/derived split [Chapter 6](crate::filters) covers for
//! filters: a relationship's method only ever writes a cell's *derived* shadow, never its
//! *source*, so a cell that stops being claimed springs back to `source` exactly as a
//! deactivated filter's cell would.
//!
//! ```rust
//! let mut parser = adam_lang_book::support::parser();
//! let mut parsed = parser
//!     .parse_str(
//!         r#"
//!         sheet no_default {
//!             cell mode: i32 = 0;
//!             cell x: f64 = 1.0;
//!
//!             conditional mode {
//!                 0i32 => {
//!                     relationship { x := 100.0; }
//!                 }
//!             }
//!         }
//!         "#,
//!     )
//!     .unwrap();
//! let mode = parsed.cell_names["mode"].0;
//! let x = parsed.cell_names["x"].0;
//!
//! parsed.propagate().unwrap();
//! assert_eq!(*parsed.read::<f64>(x).unwrap(), 100.0); // mode == 0: branch active
//! assert!(!parsed.is_source(x));
//!
//! parsed.write(mode, 7_i32).unwrap();
//! parsed.propagate().unwrap();
//! assert_eq!(*parsed.read::<f64>(x).unwrap(), 1.0); // no branch matches; x reverts to its
//!                                                    // own declared default, not 100.0
//! assert!(parsed.is_source(x));
//! ```
//!
//! ## 5.4 Nested and chained conditionals
//!
//! Two `conditional`s in the same sheet compose freely at the sheet level — one conditional's
//! output cells can be another's match subject or a relationship input, exactly like any other
//! cell — even though a branch body itself can only hold `relationship`s, not another
//! `conditional`. `begin/examples/image_resize.adm2` has two sibling conditionals this way: one
//! gated on `resample`, the other on `resample && constrain`.
