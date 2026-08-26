//! # Chapter 4: Relationships and the Solver
//!
//! ## 4.1 Bindings are alternative methods
//!
//! ```text
//! relationship_decl = "relationship" "{" { binding } "}".
//! binding            = binding_target ":=" expression ";".
//! binding_target     = identifier | "(" identifier { "," identifier } [ "," ] ")".
//! ```
//!
//! Each `binding` inside a `relationship` block is a candidate **method**: an expression whose
//! dependencies are [deduced](crate::expressions#34-deduced-dependencies) from whichever
//! already-declared cells it references, paired with the cell(s) named on its left of `:=`. A
//! relationship's bindings are alternatives, not a sequence — at any moment, exactly one of them
//! is *selected*, and only the selected one's output cell(s) are written by `propagate()`. The
//! other bindings simply aren't evaluated that round.
//!
//! ## 4.2 Strength: who gets to stay a source
//!
//! Every cell carries a **strength**, a write-recency counter: `propagate()` re-derives the
//! *stalest* cells it safely can and leaves the *freshest* cells alone. Two things bump a cell's
//! strength — an explicit host `write()`, and, once only, the cell's own declaration — so before
//! any `write()` has happened, declaration order alone orders every cell's freshness, earliest
//! declared being stalest. Chapter 1's [§1.2](crate::tutorial#12-relationships-multi-way-constraints)
//! walks through the simplest case of this rule. `write()` never touches strength itself except
//! to promote the written cell to "freshest of all" — reading a cell never changes it.
//!
//! ## 4.3 A shared-cell example
//!
//! Cells can be shared across more than one relationship, letting the solver's strength
//! preference cross relationship boundaries. Four cells, two relationships, with `b` and `c`
//! shared between them:
//!
//! ```text
//! sheet diamond {
//!     cell a = 0.0;
//!     cell b = 0.0;
//!     cell c = 2.0;
//!     cell d = 3.0;
//!
//!     relationship {
//!         c := a * b;
//!         b := c / a;
//!         a := c / b;
//!     }
//!
//!     relationship {
//!         d := b * c;
//!         c := d / b;
//!         b := d / c;
//!     }
//! }
//! ```
//!
//! Declaration order makes `d` freshest, then `c`, then `b`, then `a` stalest. The solver tries,
//! strongest first, to leave each cell a source:
//!
//! - **`d`**: the second relationship has a binding that doesn't write `d` (`b := d/c` or
//!   `c := d/b`), so `d` can stay a source.
//! - **`c`**: with `d` already pinned as a source, the second relationship's only way to avoid
//!   writing `d` is `b := d/c` — which doesn't touch `c` either, so `c` can *also* stay a
//!   source, as long as the first relationship also avoids writing it (`a := c/b` does).
//! - **`b`**: now both relationships are already spoken for in a way that avoids writing `b`
//!   only by the first relationship (`a := c/b`) — but the second relationship's only binding
//!   that doesn't write `c` or `d` is `b := d/c`, which *does* write `b`. There is no way left
//!   to leave `b` a source, so the attempt fails and `b` stays claimed by the second
//!   relationship.
//! - **`a`**: stalest, and the first relationship's remaining choice is `a := c/b` — `a` is
//!   derived.
//!
//! ```rust
//! let mut parser = adam_lang_book::support::parser();
//! let mut parsed = parser
//!     .parse_str(
//!         r#"
//!         sheet diamond {
//!             cell a = 0.0;
//!             cell b = 0.0;
//!             cell c = 2.0;
//!             cell d = 3.0;
//!
//!             relationship {
//!                 c := a * b;
//!                 b := c / a;
//!                 a := c / b;
//!             }
//!
//!             relationship {
//!                 d := b * c;
//!                 c := d / b;
//!                 b := d / c;
//!             }
//!         }
//!         "#,
//!     )
//!     .unwrap();
//! parsed.propagate().unwrap();
//!
//! let (a, b, c, d) = (
//!     parsed.cell_names["a"].0,
//!     parsed.cell_names["b"].0,
//!     parsed.cell_names["c"].0,
//!     parsed.cell_names["d"].0,
//! );
//! assert!(parsed.is_source(c) && parsed.is_source(d));
//! assert!(!parsed.is_source(a) && !parsed.is_source(b));
//! assert_eq!(*parsed.read::<f64>(c).unwrap(), 2.0); // untouched
//! assert_eq!(*parsed.read::<f64>(d).unwrap(), 3.0); // untouched
//! assert_eq!(*parsed.read::<f64>(b).unwrap(), 1.5); // d / c
//! assert_eq!(*parsed.read::<f64>(a).unwrap(), 4.0 / 3.0); // c / b
//! ```
//!
//! [`adam_rs::Sheet::is_source`] answers "did the last `propagate()` leave this cell alone" —
//! useful for a host UI deciding whether a field should be editable.
//!
//! ## 4.4 When no assignment exists
//!
//! Every relationship in a sheet must end up with exactly one selected binding once
//! `propagate()` runs — if that's not possible, `propagate()` fails instead of silently picking
//! something inconsistent. Two relationships that both, unconditionally, insist on writing the
//! *same* cell can never both be satisfied:
//!
//! ```rust
//! let mut parser = adam_lang_book::support::parser();
//! let err = parser
//!     .parse_str(
//!         r#"
//!         sheet conflict {
//!             cell x = 1.0;
//!
//!             relationship { x := 1.0; }
//!             relationship { x := 2.0; }
//!         }
//!         "#,
//!     )
//!     .unwrap()
//!     .propagate()
//!     .unwrap_err();
//! assert!(matches!(err, adam_rs::Error::Conflict));
//! ```
//!
//! A subtler failure is a **cycle**: an assignment exists, but every valid choice of bindings
//! forms a closed loop with no cell left as a source anywhere in the loop — nothing external
//! ever breaks the chain:
//!
//! ```rust
//! let mut parser = adam_lang_book::support::parser();
//! let err = parser
//!     .parse_str(
//!         r#"
//!         sheet cycle {
//!             cell x = 1.0;
//!             cell y = 1.0;
//!             cell z = 1.0;
//!
//!             relationship { y := x; }
//!             relationship { z := y; }
//!             relationship { x := z; }
//!         }
//!         "#,
//!     )
//!     .unwrap()
//!     .propagate()
//!     .unwrap_err();
//! assert!(matches!(err, adam_rs::Error::Cycle));
//! ```
//!
//! Each relationship above has only one binding, so the solver has no alternative to try —
//! `x`, `y`, and `z` are forced into a cycle regardless of strength. Giving even one of the
//! three relationships a second, cycle-breaking binding (e.g. also allowing `y := x` to run in
//! reverse as `x := y`) would let the solver route around the loop instead.
//!
//! ## 4.5 Destructuring bindings
//!
//! A binding's left-hand side can name more than one output cell by parenthesizing it, in which
//! case the right-hand side must be a tuple expression of matching arity, split element-wise:
//!
//! ```rust
//! let mut parser = adam_lang_book::support::parser();
//! let mut parsed = parser
//!     .parse_str(
//!         r#"
//!         sheet swap_demo {
//!             cell a: i32 = 1;
//!             cell b: i32 = 2;
//!
//!             relationship {
//!                 (a, b) := (b, a);
//!             }
//!         }
//!         "#,
//!     )
//!     .unwrap();
//! parsed.propagate().unwrap();
//! let (a, b) = (parsed.cell_names["a"].0, parsed.cell_names["b"].0);
//! assert_eq!(*parsed.read::<i32>(a).unwrap(), 2);
//! assert_eq!(*parsed.read::<i32>(b).unwrap(), 1);
//! ```
//!
//! `(a, b) := ...` and the one-element `(a,) := ...` (trailing comma mandatory, matching Rust's
//! own 1-tuple pattern) both destructure; a bare `a := ...` or the equivalent single
//! parenthesized `(a) := ...` (mere grouping, no comma) instead binds the right-hand side's
//! *whole* result — including a tuple-typed one — directly to the one named cell. Destructuring
//! and direct-bind are otherwise governed by the same type-matching rules as any other binding:
//! each output's declared type must structurally match what the expression actually produces,
//! checked at parse time.
