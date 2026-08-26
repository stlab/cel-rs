//! # Chapter 6: Filters — Self-Correcting Cells
//!
//! ## 6.1 Grammar
//!
//! ```text
//! cell_filter = "filter" expression.
//! ```
//!
//! A `filter` clause is optional and trails a `cell` declaration's type/initializer. Its
//! `expression` is [deduced](crate::expressions#34-deduced-dependencies) exactly like a
//! relationship binding's, plus one reserved identifier: `_` always refers to the *candidate
//! value being conformed* — of the filtered cell's own declared type — never a cell. `_` is
//! reserved inside a filter expression only; outside one it's an ordinary identifier (or the
//! [conditional](crate::conditionals) default-branch token).
//!
//! ```text
//! cell level: i32 = 50 filter 0..=100;     // a fixed range
//! cell level: i32 = 50 filter 0..=max;     // a range whose upper bound is another cell
//! cell level: i32 = 50 filter clamp(_, 0, max);  // an arbitrary expression over `_`
//! ```
//!
//! A filter expression must reference `_` at least once (unless it's a range expression — see
//! 6.4) and must produce a value of exactly the filtered cell's own type; violating either is a
//! parse-time error, not a runtime one.
//!
//! ## 6.2 `write()` never filters
//!
//! This is the single most important rule in this chapter: **`write()` always stores exactly
//! the value it was given**, filter or no filter. A filter is applied live, by `propagate()`,
//! against the cell's own current value — never synchronously inside `write()`.
//!
//! ```rust
//! let mut parser = adam_lang_book::support::parser();
//! let mut parsed = parser
//!     .parse_str("sheet s { cell level: i32 = 50 filter 0..=100; }")
//!     .unwrap();
//! let level = parsed.cell_names["level"].0;
//!
//! parsed.write(level, 500_i32).unwrap();
//! assert_eq!(*parsed.read::<i32>(level).unwrap(), 500); // the raw value, unfiltered
//!
//! parsed.propagate().unwrap();
//! assert_eq!(*parsed.read::<i32>(level).unwrap(), 100); // now conformed
//! ```
//!
//! `write()` cannot fail because of a filter, and never returns an error on a filter's account.
//! Whatever you write is exactly what `read()` shows until the next `propagate()` — the same
//! "read reflects the last full `propagate()`, not a per-write side effect" rule every other
//! cell in a sheet already follows.
//!
//! ## 6.3 The raw value is never lost
//!
//! A filtered cell keeps two shadow values under the hood: its raw last-written value
//! (`adam_rs` calls this a cell's `source`) and, when something currently claims it, a computed
//! override (`derived`) — [`adam_rs::Sheet::read`] always returns `derived` if present, `source`
//! otherwise. A filter's live output always lands in `derived`, **never** in `source` — so a
//! filtered cell's original input is never destroyed, even after many rounds of clamping. If a
//! dynamic bound loosens back up, the cell springs back to exactly what was last written, not
//! to some intermediate clamped value:
//!
//! ```rust
//! let mut parser = adam_lang_book::support::parser();
//! let mut parsed = parser
//!     .parse_str(
//!         r#"
//!         sheet spring_back {
//!             cell max: i32 = 100 filter 0..=200;
//!             cell level: i32 = 50 filter 0..=max;
//!         }
//!         "#,
//!     )
//!     .unwrap();
//! let (max, level) = (parsed.cell_names["max"].0, parsed.cell_names["level"].0);
//!
//! parsed.write(max, 10_i32).unwrap();
//! parsed.propagate().unwrap();
//! assert_eq!(*parsed.read::<i32>(level).unwrap(), 10); // clamped down
//!
//! parsed.write(max, 100_i32).unwrap();
//! parsed.propagate().unwrap();
//! assert_eq!(*parsed.read::<i32>(level).unwrap(), 50); // back to the original 50, not 10
//! ```
//!
//! This is the same rule [Chapter 5](crate::conditionals#53-the-default-branch) already showed
//! for a relationship's method: a method's output — and a filter's output — always writes
//! `derived`, so nothing a *computation* produces can ever permanently overwrite what was
//! actually written.
//!
//! ## 6.4 Range filters
//!
//! A filter expression whose type is CEL's `lo..=hi` range (over any type this book's
//! [built-in numeric types](crate::cells#23-built-in-types-and-inference) supports) is
//! recognized structurally as a **range filter**: `propagate()` clamps into `[lo, hi]` instead
//! of running the expression as an arbitrary function of `_`, and the sheet can report the
//! range's current live bounds without needing a candidate value at all:
//!
//! ```rust
//! let mut parser = adam_lang_book::support::parser();
//! let mut parsed = parser
//!     .parse_str("sheet s { cell level: i32 = 50 filter 0..=100; }")
//!     .unwrap();
//! let level = parsed.cell_names["level"].0;
//! assert!(matches!(
//!     parsed.filter_kind(level),
//!     Some(adam_rs::FilterKind::Range { .. })
//! ));
//! assert_eq!(parsed.filter_range::<i32>(level), Some((0, 100)));
//! ```
//!
//! A range filter's body is exempt from the "must reference `_`" rule (6.1) — a genuine range
//! expression like `0..=max` has no reason to mention `_` at all, since both endpoints are
//! independent of the value being conformed.
//!
//! ## 6.5 Derived cells: diagnosed, never corrected
//!
//! A filter attaches to *one* cell, but that cell isn't always a source — a relationship may
//! claim it instead (Chapter 4). When that happens, the filter no longer has any authority to
//! change the value: it only *observes*. `propagate()` still succeeds, and the out-of-range
//! value is still what `read()` returns; the sheet simply records that the filter is violated:
//!
//! ```rust
//! let mut parser = adam_lang_book::support::parser();
//! let mut parsed = parser
//!     .parse_str(
//!         r#"
//!         sheet diagnose_only {
//!             cell bound: i32 = 100 filter 0..=100;
//!             cell driver: i32 = 500;
//!
//!             relationship {
//!                 bound := driver;
//!             }
//!         }
//!         "#,
//!     )
//!     .unwrap();
//! parsed.propagate().unwrap();
//!
//! let bound = parsed.cell_names["bound"].0;
//! assert_eq!(*parsed.read::<i32>(bound).unwrap(), 500); // not clamped
//! assert!(parsed.filter_violated_cells().any(|id| id == bound));
//! ```
//!
//! `propagate()` never fails because of a filter violation, on either side (source or derived)
//! — a filter is a diagnostic and a self-correction mechanism, never a gate. See
//! [`adam_rs::Sheet::filter_violation`], [`adam_rs::Sheet::filter_violated_cells`], and
//! [`adam_rs::Sheet::filter_violation_cells`] for the query API a host UI uses to surface this.
//!
//! ## 6.6 Errors
//!
//! Every filter error below is caught while parsing the sheet, before `propagate()` ever runs:
//!
//! ```rust
//! let mut parser = adam_lang_book::support::parser();
//! let err = parser
//!     .parse_str("sheet s { cell x: i32 = 0 filter 5; }") // never mentions `_`
//!     .err()
//!     .unwrap();
//! assert!(format!("{err}").contains("must reference `_`"));
//! ```
//!
//! ```rust
//! let mut parser = adam_lang_book::support::parser();
//! let err = parser
//!     .parse_str("sheet s { cell x: (i32, i32) = (0, 0) filter _; }") // tuple-typed cell
//!     .err()
//!     .unwrap();
//! assert!(format!("{err}").contains("tuple"));
//! ```
//!
//! At most one filter may be attached per cell, and a filter cannot attach to an
//! [output](crate::outputs) cell (outputs already have `require` for diagnostics, and can never
//! be written directly, so a filter's write-time half would be moot there).
