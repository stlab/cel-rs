//! # adam-lang
//!
//! A DSL parser for property models. Parses an adam-lang source string and produces
//! a live [`ParsedSheet`] (sheet plus cell names in declaration order).
//!
//! # Grammar
//!
//! ```text
//! module             = { use_decl | sheet }.
//! use_decl           = "use" qualified_name [ "as" identifier ] ";".
//! sheet              = "sheet" identifier [ "(" [ parameter { "," parameter } ] ")" ]
//!                        "{" { sheet_item } "}".
//! parameter          = identifier ":" type_expr.
//! sheet_item         = [ doc_comment ] (cell_decl | relationship_decl | conditional_decl | out_decl
//!                        | source_decl | relationship_use).
//! cell_decl          = "cell" identifier cell_type_init [ cell_filter ] [ require_block ] ";".
//! cell_filter        = "filter" expression.
//! cell_type_init     = (":" type_expr ["=" expression]) | ("=" expression).
//! source_decl        = "source" identifier cell_type_init [ cell_filter ] [ require_block ] ";".
//! type_expr          = identifier [ "(" type_expr { "," type_expr } ")" ]
//!                    | "[" type_expr "]"
//!                    | "(" [ type_expr ["," [ type_expr { "," type_expr } ]] ] ")".
//! relationship_decl  = "relationship" "{" { binding } "}".
//! relationship_use   = "relationship" qualified_name "(" [ actual { "," actual } ] ")" ";".
//! actual             = identifier ":" identifier.
//! qualified_name     = identifier { "::" identifier }.
//! binding            = binding_target ":=" expression ";".
//! binding_target     = identifier | "(" identifier "," [ identifier { "," identifier } ] ")".
//! conditional_decl   = "conditional" expression "{" { conditional_branch } "}".
//! conditional_branch = (literal_pattern | "_") "=>" "{"
//!                        { relationship_decl | relationship_use } "}".
//! out_decl           = "out" identifier [":" type_expr] ":=" expression
//!                        [ cell_filter ] [ require_block ] ";".
//! require_block      = "require" "{" { requirement } "}".
//! requirement        = [ "@" identifier ] expression ";".
//! ```
//!
//! Every production uses strict LL(1) parsing: a comma commits to another item.
//! Only singleton tuple values, types and destructuring bindings require a terminal comma.
//! Conditional branches have no comma separator or terminator.
//!
//! A sheet without a parameter list is a root; a sheet with a parameter list
//! defines a reusable component. [`AdamParser::parse_str`] constructs a module's
//! sole root through [`ModuleCompiler`]. Collections with several roots use
//! [`CompiledModules::instantiate_root`] for explicit selection. Construction
//! checks the final combined graph without propagation. [`check_module`] checks
//! registered sources and imports without executing any expressions.
//!
//! # Reusable components and explicit roots
//!
//! A component aliases caller-owned cells; it does not copy their values. Formal
//! parameters have concrete types and no initializers, filters or requirements.
//! Each use supplies every formal exactly once, in any named order, with distinct
//! actual cells of exactly matching complete types. Actuals may be ordinary cells
//! or fixed sources, never `out` cells, expressions or literals. A component may
//! allocate private cells and diagnostic outputs, but cannot attach policies to
//! parameters or turn parameters into outputs. The caller owns boundary policies.
//!
//! The following example registers both the library and importing root. It uses
//! builtin arithmetic only; hosts needing standard or custom functions install
//! their operation libraries before moving the lookup into the immutable compiler.
//!
//! ```
//! use adam_lang::{ModuleCompiler, ModuleSet, TypeRegistry};
//! let mut modules = ModuleSet::new();
//! modules.insert_source("numbers".parse()?, "numbers.adm2", r#"
//!     sheet multiply(a: f64, b: f64, product: f64) {
//!         relationship {
//!             product := a * b;
//!             a := product / b;
//!             b := product / a;
//!         }
//!     }
//! "#)?;
//! modules.insert_source("app".parse()?, "app.adm2", r#"
//!     use numbers::multiply as mul;
//!     sheet app {
//!         source factor: f64 = 3.0;
//!         cell value: f64 = 2.0;
//!         cell result: f64 = 0.0;
//!         relationship mul(product: result, b: factor, a: value);
//!     }
//!     sheet other {}
//! "#)?;
//! let compiled = ModuleCompiler::new(TypeRegistry::new(), cel_parser::OpLookup::new())
//!     .compile(modules).map_err(|mut errors| errors.remove(0))?;
//! let path = "app".parse()?;
//! assert_eq!(compiled.roots(&path).collect::<Vec<_>>(), ["app", "other"]);
//! let mut parsed = compiled.instantiate_root(&path, "app")?;
//! drop(compiled); // Live methods and diagnostics retain their own resources.
//! let value = parsed.cell_names["value"].0;
//! let result = parsed.cell_names["result"].0;
//! assert_eq!(*parsed.read::<f64>(result).unwrap(), 0.0); // No propagation yet.
//! parsed.sheet.write(value, 7.0f64).unwrap();
//! parsed.propagate().unwrap();
//! assert_eq!(*parsed.read::<f64>(result).unwrap(), 21.0);
//! parsed.sheet.write(result, 12.0f64).unwrap();
//! parsed.propagate().unwrap();
//! assert_eq!(*parsed.read::<f64>(value).unwrap(), 4.0);
//! # Ok::<(), adam_lang::ModuleDiagnostic>(())
//! ```
//!
//! Source specialization removes methods that write fixed sources, including
//! through nested uses. It does not infer equations or inspect current values.
//! An empty specialized relationship fails construction, even in an inactive
//! conditional branch. Conditional uses must contain only relationships
//! transitively: private cells, outputs and policies make a component ineligible.
//! Unconditional uses may contain those private declarations.
//!
//! Compilation checks every definition without executing host expressions;
//! construction runs defaults, initializers and factories but does not propagate.
//! Runtime division and conversion errors still require handling. Requirements
//! report validity, not propagation failure; inspect
//! [`Sheet::cell_requirements_valid`](adam_rs::Sheet::cell_requirements_valid).
//! Native instantiation failures leave the destination safely discardable, not
//! necessarily unchanged. Owning [`ModuleDiagnostic`] labels identify real files;
//! [`ParsedSheet::locate_error`] resolves runtime errors to definition/use sites.
//! No filesystem loader, implicit module search or generic type parameter exists.
//! Imports name declared components, not roots or imported aliases.
//!
//! The design spec for `cell_decl` also calls for an optional trailing `":=" expression`
//! clause, not shown above because **this crate does not yet implement it** — see
//! `docs/superpowers/specs/2026-08-19-adam-lang-syntax-design.md`'s "Explicitly out of scope"
//! section; it's deferred pending a forward-reference/hoisting decision. Only `cell_decl`'s
//! `"=" expression` one-time initializer and its optional `cell_filter`/`require_block`
//! clauses are implemented today.
//!
//! `expression` and its descendants (`literal`, `identifier`, and the rest of the
//! CEL expression grammar), as well as `literal_pattern` (a bare literal, optionally negated by
//! a leading `-`, matching Rust's own `LiteralPattern` rule — CEL has no constant-expression
//! syntax in pattern position), are defined by `cel_parser` — see that crate's own
//! [`# Grammar`](../cel_parser/index.html#grammar) section. That embedded CEL grammar now includes
//! postfix array type ascriptions such as `[left, right]: [Custom]` and `[]: [[i32]]`. The
//! ascription names the complete array type, resolves leaf names through the active
//! [`TypeRegistry`], and reuses recursive tuple/array/callable type syntax. Persistent cells
//! support arbitrary compositions of these shapes and registered generic leaves. Arrays
//! default to empty, tuples default recursively, and callables require an initializer.
//! Every stored cell requires cloning and value equality: CEL permits callable storage, but
//! Adam rejects a callable cell without equality rather than inventing function equality.
//! Registered concrete leaves keep [`Sheet`](adam_rs::Sheet)'s Rust read/write access;
//! compound owners use [`ParsedSheet::read_compound`], [`ParsedSheet::write_compound`], and
//! [`ParsedSheet::source_compound`] with canonical representation witnesses. Relationships,
//! filters, outputs, conditionals, and requirements validate complete frames at construction
//! and run the existing raw CEL engine without recursively checking types on propagation.
//! The private `CellValue` owns one complete compound value, not individual erased
//! array elements. Checked host writes reject mismatched recursive witnesses before
//! mutation. Reads clone independent owners; source reads preserve aspirations.
//! Formatting is a separate capability: [`prepare_compound_cell_formatter`] binds
//! it without adding equality or clone requirements.
//!
//! ```
//! use adam_lang::{AdamParser, TypeRegistry};
//! use cel_parser::OpLookup;
//! use cel_runtime::{Array, ArrayOf, Cons, IntoCStackList, Leaf, Nil};
//! type Rows = ArrayOf<Cons<Leaf<i32>, Cons<Leaf<bool>, Nil>>>;
//! let mut parsed = AdamParser::new(TypeRegistry::new(), OpLookup::new())
//!     .parse_str("sheet s { source rows: [(i32, bool)]; out copy := rows; }").unwrap();
//! let id = parsed.cell_names["rows"].0;
//! parsed.write_compound::<Rows>(id, Array::from_vec(vec![(7, true).into_c_stack_list()])).unwrap();
//! parsed.propagate().unwrap();
//! assert_eq!(parsed.read_compound::<Rows>(parsed.output_names["copy"]).unwrap().len(), 1);
//! assert_eq!(parsed.source_compound::<Rows>(id).unwrap().as_slice()[0].1, 7);
//! ```
//!
//! A `cell_filter`'s `expression` names no explicit parameter list: `_` always refers to the
//! candidate value being conformed (of the filtered cell's own declared type), and every other
//! identifier that names an already-declared cell is a dependency, deduced exactly as a
//! `relationship` binding's/`out` declaration's/`conditional`'s own `expression` deduces its
//! inputs — see [`ast::CellFilter`]. `_` is reserved inside a filter expression only.
//!
//! # Example
//!
//! `AdamParser::new` takes an [`OpLookup`](cel_parser::OpLookup) instance. See
//! [`OpLookup::push_library_scope`](cel_parser::OpLookup::push_library_scope) for how to
//! install one (e.g. `cel-std`) before parsing.
//!
//! ```rust
//! use adam_lang::{AdamParser, TypeRegistry};
//! use cel_parser::OpLookup;
//!
//! let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
//! let parsed = parser.parse_str(r#"
//!     sheet image_resize {
//!         cell width:  f64 = 1920.0;
//!         cell height: f64 = 1080.0;
//!         cell area:   f64;
//!     }
//! "#).unwrap();
//! ```
//!
//! Built-in and registered Adam types are both visible inside embedded CEL array ascriptions:
//!
//! ```rust
//! use adam_lang::{AdamParser, TypeRegistry};
//! use cel_parser::OpLookup;
//! use cel_runtime::{ArrayOf, Leaf, ValueRepr};
//!
//! #[derive(Clone, Debug, PartialEq)]
//! struct Custom(i32);
//!
//! #[derive(Clone)]
//! struct CountFn;
//!
//! let mut lookup = OpLookup::new();
//! lookup.push_scope(|name, segment, arity, _span| match (name, arity) {
//!     ("left", 0) => {
//!         segment.op0(|| Custom(1));
//!         Ok(true)
//!     }
//!     ("right", 0) => {
//!         segment.op0(|| Custom(2));
//!         Ok(true)
//!     }
//!     ("count", 0) => {
//!         segment.op0(|| CountFn);
//!         Ok(true)
//!     }
//!     ("()", 2) => {
//!         let array = &segment.peek_stack_infos(1)[0].value_type;
//!         if array.same_shape(&ArrayOf::<Leaf<i32>>::value_type()) {
//!             segment.op2_repr::<Leaf<CountFn>, ArrayOf<Leaf<i32>>, Leaf<i32>, _>(
//!                 |_callee, values| values.len() as i32)?;
//!         } else if array.same_shape(&ArrayOf::<Leaf<Custom>>::value_type()) {
//!             segment.op2_repr::<Leaf<CountFn>, ArrayOf<Leaf<Custom>>, Leaf<i32>, _>(
//!                 |_callee, values| values.len() as i32)?;
//!         } else {
//!             segment.op2_repr::<Leaf<CountFn>, ArrayOf<ArrayOf<Leaf<Custom>>>, Leaf<i32>, _>(
//!                 |_callee, values| values.len() as i32)?;
//!         }
//!         Ok(true)
//!     }
//!     _ => Ok(false),
//! });
//!
//! let mut types = TypeRegistry::new();
//! types.register_no_default::<Custom>("Custom");
//! let mut parser = AdamParser::new(types, lookup);
//! let parsed = parser.parse_str(
//!     "sheet s { \
//!         cell scalar: i32 = count([1, 2]: [i32]); \
//!         cell values: i32 = count([left, right]: [Custom]); \
//!         cell empty: i32 = count([]: [Custom]); \
//!         cell nested: i32 = count([[]: [Custom]]: [[Custom]]); \
//!     }",
//! ).unwrap();
//!
//! let (scalar, _) = parsed.cell_names["scalar"];
//! assert_eq!(*parsed.read::<i32>(scalar).unwrap(), 2);
//! let (values, _) = parsed.cell_names["values"];
//! assert_eq!(*parsed.read::<i32>(values).unwrap(), 2);
//! let (empty, _) = parsed.cell_names["empty"];
//! assert_eq!(*parsed.read::<i32>(empty).unwrap(), 0);
//! let (nested, _) = parsed.cell_names["nested"];
//! assert_eq!(*parsed.read::<i32>(nested).unwrap(), 1);
//! ```
//!
//! Embedded CEL array diagnostics stay exact:
//!
//! ```rust
//! use adam_lang::{AdamParser, TypeRegistry};
//! use cel_parser::OpLookup;
//!
//! let mut parser = AdamParser::new(TypeRegistry::new(), OpLookup::new());
//! let err = parser
//!     .parse_str("sheet s { cell values = []: [Missing]; }")
//!     .unwrap_err();
//! assert_eq!(err.message(), "unknown type `Missing`");
//! ```
//!
//! ```rust
//! use adam_lang::{AdamAstParser, TypeRegistry, check_sheet};
//!
//! let sheet = AdamAstParser::new()
//!     .parse_str("sheet s { out values := [0, 1]: [f64]; }")
//!     .unwrap();
//! let diagnostics = check_sheet(&sheet, &TypeRegistry::new(), &cel_parser::OpLookup::new());
//! assert_eq!(diagnostics.len(), 1);
//! assert_eq!(
//!     diagnostics[0].message(),
//!     "array elements must match the annotation exactly: expected `f64`, found `i32`"
//! );
//! ```

pub mod ast;
mod ast_parser;
mod cell_value;
pub mod composition;
mod error_labels;
mod fmt;
pub mod module;
mod parser;
pub mod source_map;
mod token_cursor;
mod trivia;
pub mod type_registry;
mod typecheck;

// Live-root errors keep their owning ModuleDiagnostic in ParseError's source chain.
pub use ast_parser::AdamAstParser;
pub use cel_parser::ParseError;
pub use cell_value::{debug_compound_cell, prepare_compound_cell_formatter};
pub use composition::{
    AuthoredItem, AuthoredRole, CellOrigin, CellSlot, CompositionBuilder, InstanceOrigin,
    MethodRecipe, OccurrenceOrigin, TypedTemplate,
};
pub use fmt::{FormatSourceError, format_module, format_sheet, format_source};
pub use module::{
    CompiledModules, ModuleCompiler, ModulePath, ModuleSet, PreparedRoot, RootCell, check_module,
};
pub use parser::{AdamParser, ParsedSheet};
pub use source_map::{
    ModuleCause, ModuleDiagnostic, SourceId, SourceLabel, SourceLocation, SourceMap,
};
pub use trivia::{attach_module_trivia, attach_trivia};
pub use type_registry::TypeRegistry;
pub use typecheck::check_sheet;
