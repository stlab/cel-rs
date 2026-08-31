//! # Chapter 1: A Tutorial Introduction
//!
//! Let's get started. The best way to learn a new language is to write programs in it, and
//! adam-lang programs are called **sheets**. This chapter is a fast, informal tour of every
//! construct adam-lang has; later chapters go back over the same ground in more detail, and the
//! [reference manual](crate::reference) collects the precise rules for looking things up.
//!
//! You don't need to install anything to follow along: every source fragment below either
//! stands on its own as a `.adm2` file, or is a complete, compiled, and tested Rust program
//! (`cargo test --doc` runs every one of them) showing how a host application feeds that source
//! to adam-lang and reads the results back out.
//!
//! ## 1.1 A first sheet
//!
//! An adam-lang program is a single `sheet`, named, with a body of declarations between braces.
//! The simplest useful sheet declares a few cells and nothing else:
//!
//! ```text
//! sheet hello {
//!     cell width: i32 = 1920;
//!     cell height: i32 = 1080;
//! }
//! ```
//!
//! A **cell** is a named, typed storage location — the basic unit of state in a property
//! model. `width` and `height` are `i32`-typed cells, each given an initial value.
//! Semicolons end declarations, exactly as in Rust or C; a sheet's body is a sequence of
//! declarations, not a sequence of statements — there is no control flow at this level, no
//! loops, and no imperative execution order. A sheet describes a *graph* of cells and the
//! constraints between them, not a sequence of steps to run.
//!
//! To do anything with a sheet, a host program parses it, then reads and writes cells by
//! `CellId`, exactly like driving `adam_rs::Sheet` directly:
//!
//! ```rust
//! let mut parser = adam_lang_book::support::parser();
//! let parsed = parser
//!     .parse_str(
//!         r#"
//!         sheet hello {
//!             cell width: i32 = 1920;
//!             cell height: i32 = 1080;
//!         }
//!         "#,
//!     )
//!     .unwrap();
//!
//! let width = parsed.cell_names["width"].0;
//! assert_eq!(*parsed.read::<i32>(width).unwrap(), 1920);
//! ```
//!
//! `parser.parse_str` returns a [`adam_lang::ParsedSheet`], which derefs to [`adam_rs::Sheet`] —
//! every `Sheet` method (`read`, `write`, `propagate`, and the rest) works directly on it — plus
//! a `cell_names` table mapping each declared name to its `CellId`, so a host application can
//! look cells up by the name the sheet author gave them.
//!
//! ## 1.2 Relationships: multi-way constraints
//!
//! A sheet with only cells and no relationships is just a struct. What makes adam-lang
//! interesting is the **relationship**: a set of alternative ways to keep a group of cells
//! consistent, any one of which the solver may pick at any given moment.
//!
//! The classic example is three numbers related by multiplication — `a * b = c` — where any one
//! of the three can be computed from the other two:
//!
//! ```text
//! sheet triangle {
//!     cell c = 0.0;
//!     cell a = 2.0;
//!     cell b = 3.0;
//!
//!     relationship {
//!         c := a * b;
//!         a := c / b;
//!         b := c / a;
//!     }
//! }
//! ```
//!
//! The `relationship` block offers three **bindings** — `c := a * b`, `a := c / b`, and
//! `b := c / a` — each an alternative *method* for deriving one cell from the others. Only one
//! binding is active at a time; which one is chosen depends on which cells were written most
//! recently (see [Chapter 4](crate::relationships) for the full rule). A cell's *declaration*
//! counts as a write for this purpose, so before anything is ever explicitly written, cells
//! declared earlier are treated as "staler" than cells declared later. The solver prefers to
//! leave the freshest cells alone and derive the stalest one — here, `c`, declared first:
//!
//! ```rust
//! let mut parser = adam_lang_book::support::parser();
//! let mut parsed = parser
//!     .parse_str(
//!         r#"
//!         sheet triangle {
//!             cell c = 0.0;
//!             cell a = 2.0;
//!             cell b = 3.0;
//!
//!             relationship {
//!                 c := a * b;
//!                 a := c / b;
//!                 b := c / a;
//!             }
//!         }
//!         "#,
//!     )
//!     .unwrap();
//! parsed.propagate().unwrap();
//!
//! let (a, b, c) = (
//!     parsed.cell_names["a"].0,
//!     parsed.cell_names["b"].0,
//!     parsed.cell_names["c"].0,
//! );
//! assert_eq!(*parsed.read::<f64>(c).unwrap(), 6.0); // 2.0 * 3.0, derived
//!
//! // Write b: it becomes the freshest cell, so the solver keeps both a and b as
//! // sources and re-derives c from them.
//! parsed.write(b, 5.0).unwrap();
//! parsed.propagate().unwrap();
//! assert_eq!(*parsed.read::<f64>(a).unwrap(), 2.0); // untouched
//! assert_eq!(*parsed.read::<f64>(b).unwrap(), 5.0); // just written
//! assert_eq!(*parsed.read::<f64>(c).unwrap(), 10.0); // 2.0 * 5.0, re-derived
//! ```
//!
//! Nothing here names *which* cell is the "output" — that's the whole point. Whichever cell was
//! written (or, failing that, declared) least recently is the one the solver derives;
//! `write`ing a cell is what tells the solver "trust this one; recompute something else
//! instead."
//!
//! ## 1.3 Conditionals
//!
//! A `conditional` groups relationships that are only active under a matching condition.  It
//! evaluates a **match subject**, then activates whichever branch's literal equals the current
//! match value:
//!
//! ```text
//! sheet mode_demo {
//!     cell p: i32 = 0;
//!     cell x: f64 = 1.0;
//!     cell y: f64 = 2.0;
//!
//!     conditional p {
//!         0i32 => {
//!             relationship {
//!                 x := y;
//!             }
//!         }
//!         1i32 => {
//!             relationship {
//!                 y := x;
//!             }
//!         }
//!         _ => {
//!             relationship {
//!                 x := 0.0;
//!             }
//!         }
//!     }
//! }
//! ```
//!
//! Only the active branch's relationships participate in that round's solve; every other
//! branch's relationships are as if they weren't declared at all. The `_` branch, if present,
//! catches any value none of the named branches list, and must be written last:
//!
//! ```rust
//! let mut parser = adam_lang_book::support::parser();
//! let mut parsed = parser
//!     .parse_str(
//!         r#"
//!         sheet mode_demo {
//!             cell p: i32 = 0;
//!             cell x: f64 = 1.0;
//!             cell y: f64 = 2.0;
//!
//!             conditional p {
//!                 0i32 => {
//!                     relationship {
//!                         x := y;
//!                     }
//!                 }
//!                 1i32 => {
//!                     relationship {
//!                         y := x;
//!                     }
//!                 }
//!                 _ => {
//!                     relationship {
//!                         x := 0.0;
//!                     }
//!                 }
//!             }
//!         }
//!         "#,
//!     )
//!     .unwrap();
//!
//! let p = parsed.cell_names["p"].0;
//! let x = parsed.cell_names["x"].0;
//!
//! parsed.propagate().unwrap();
//! assert_eq!(*parsed.read::<f64>(x).unwrap(), 2.0); // p == 0: x := y
//!
//! parsed.write(p, 2_i32).unwrap();
//! parsed.propagate().unwrap();
//! assert_eq!(*parsed.read::<f64>(x).unwrap(), 0.0); // p matches no named branch: default
//! ```
//!
//! See [Chapter 5](crate::conditionals) for branch types, tuple match subjects, and what
//! happens when no branch matches and there's no default.
//!
//! ## 1.4 Filters: self-correcting cells
//!
//! A `filter` clause attaches a standing domain constraint to a cell — most commonly, a range:
//!
//! ```text
//! sheet volume {
//!     cell level: i32 = 50 filter 0..=100;
//! }
//! ```
//!
//! Write an out-of-range value and the cell keeps it, raw, until the next `propagate()` —
//! `write()` never inspects a filter. `propagate()` is what conforms the value:
//!
//! ```rust
//! let mut parser = adam_lang_book::support::parser();
//! let mut parsed = parser
//!     .parse_str("sheet volume { cell level: i32 = 50 filter 0..=100; }")
//!     .unwrap();
//! let level = parsed.cell_names["level"].0;
//!
//! parsed.write(level, 500_i32).unwrap();
//! assert_eq!(*parsed.read::<i32>(level).unwrap(), 500); // still raw
//!
//! parsed.propagate().unwrap();
//! assert_eq!(*parsed.read::<i32>(level).unwrap(), 100); // now clamped
//! ```
//!
//! A filter's bounds don't have to be constants — `0..=max` references another cell, and the
//! clamp tracks it live. [Chapter 6](crate::filters) covers filters in full, including the
//! precise source/derived model behind "the cell keeps its own raw value forever, and the
//! filter only ever corrects what you *read*."
//!
//! ## 1.5 Tuples
//!
//! A binding's left-hand side can destructure a tuple-valued expression into several cells at
//! once, using the same `(a, b)` syntax Rust uses for tuple patterns:
//!
//! ```text
//! sheet swap_demo {
//!     cell a: i32 = 1;
//!     cell b: i32 = 2;
//!
//!     relationship {
//!         (a, b) := (b, a);
//!     }
//! }
//! ```
//!
//! Cells themselves can be tuple-typed too (`cell point: (f64, f64) = (0.0, 0.0);`); see
//! [Chapter 2](crate::cells) for tuple type syntax and [Chapter 4](crate::relationships) for the
//! destructuring-vs-direct-bind distinction.
//!
//! ## 1.6 Outputs and requirements
//!
//! An `out` declaration computes one final, read-only value from the rest of the sheet, and can
//! carry named `require`ments — boolean checks reported after every `propagate()`, never
//! enforced by rejecting a write:
//!
//! ```text
//! sheet area_demo {
//!     cell width: i32 = 10;
//!     cell height: i32 = 20;
//!
//!     out area: i32 := width * height require {
//!         not_too_big: area <= 300;
//!     };
//! }
//! ```
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
//!
//! let output = parsed.output_names["area"];
//! assert!(parsed.output_valid(output)); // 10 * 20 == 200 <= 300
//!
//! let width = parsed.cell_names["width"].0;
//! parsed.write(width, 50_i32).unwrap();
//! parsed.propagate().unwrap();
//! assert!(!parsed.output_valid(output)); // 50 * 20 == 1000 > 300
//! ```
//!
//! See [Chapter 7](crate::outputs) for the full rules: an output's cell is terminal (nothing
//! may write it directly), and a failed requirement never stops `propagate()` from succeeding —
//! it's a diagnostic, not a gate.
//!
//! ## 1.7 Comments
//!
//! `//` starts a line comment; `/* ... */` a block comment, exactly as in C, Rust, or CEL.
//! `///` immediately before a declaration and `//!` immediately before the `sheet` keyword are
//! doc comments, carried through by the language server and formatter but otherwise inert:
//!
//! ```text
//! //! A sheet describing a simple resize dialog.
//! sheet image_resize {
//!     /// The image's width in pixels, before any resampling.
//!     cell width_pixels: i32 = 1920;
//! }
//! ```
//!
//! See [Chapter 8](crate::style) for the formatter's canonical layout.
//!
//! ## 1.8 Where to go next
//!
//! That's the whole language. [Chapter 2](crate::cells) onward covers each construct in the
//! depth this chapter skipped past, and the [reference manual](crate::reference) gives you the
//! full grammar and every built-in type in one place. `begin/examples/*.adm2` in the `cel-rs`
//! workspace has several complete, larger sheets — `image_resize.adm2` in particular is a full
//! port of a real-world Adobe Photoshop-style resize dialog, worth reading once the constructs
//! below are familiar.
