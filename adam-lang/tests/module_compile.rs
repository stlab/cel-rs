//! Exercises owning module factories without the live-root convenience parser.

use adam_lang::{
    CompositionBuilder, ModuleCompiler, ModulePath, ModuleSet, ParsedSheet, TypeRegistry,
    type_registry::TypeShape,
};
use cel_parser::OpLookup;
use std::{any::TypeId, cell::Cell};

/// Returns the complete integer shape.
fn int() -> TypeShape {
    TypeShape::Named(TypeId::of::<i32>())
}

/// Registers two caller-owned integer cells.
fn destination() -> (ParsedSheet, adam_rs::CellId, adam_rs::CellId) {
    let mut sheet = ParsedSheet::new();
    let a = sheet.add_cell(3i32);
    let b = sheet.add_cell(0i32);
    let types = TypeRegistry::new();
    sheet.register_cell("a", a, int(), &types).unwrap();
    sheet.register_cell("b", b, int(), &types).unwrap();
    (sheet, a, b)
}

/// Resolves an imported wrapper in its defining module, even after cross-compilation.
#[test]
fn compiled_wrapper_keeps_defining_module_and_source_owners() {
    let library: ModulePath = "library".parse().unwrap();
    let caller: ModulePath = "caller".parse().unwrap();
    let mut modules = ModuleSet::new();
    modules
        .insert_source(
            library.clone(),
            "library.adm2",
            "sheet wrapper(a: i32, b: i32) { relationship helper(a: a, b: b); }
         sheet helper(a: i32, b: i32) { relationship { b := a + 4; } }",
        )
        .unwrap();
    modules
        .insert_source(
            caller.clone(),
            "caller.adm2",
            "use library::wrapper as imported;
         sheet helper(a: i32, b: i32) { relationship { b := a + 6; } }
         sheet call(a: i32, b: i32) { relationship imported(b: b, a: a); }
         sheet zero() {} sheet hidden() { cell secret: i32; }
         sheet first { cell visible: i32; relationship zero(); relationship hidden(); }
         sheet second { source own: i32 = 2; out answer := own; }",
        )
        .unwrap();
    modules
        .insert_source("empty".parse().unwrap(), "empty.adm2", "")
        .unwrap();
    let compiled = ModuleCompiler::new(TypeRegistry::new(), OpLookup::new())
        .compile(modules)
        .unwrap();
    assert!(
        compiled
            .sources()
            .iter()
            .any(|source| source.file_name() == "empty.adm2")
    );
    assert!(compiled.template(&caller, "first").is_none());
    assert_eq!(
        compiled.roots(&caller).collect::<Vec<_>>(),
        ["first", "second"]
    );
    assert_eq!(
        compiled
            .prepared_root(&caller, "first")
            .unwrap()
            .cells()
            .iter()
            .map(|cell| cell.name.as_str())
            .collect::<Vec<_>>(),
        ["visible"]
    );
    assert!(compiled.prepared_root(&caller, "absent").is_none());
    assert!(compiled.template(&caller, "imported").is_none());
    let root = compiled.prepared_root(&caller, "second").unwrap();
    assert_eq!(
        root.cells()
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        ["own", "answer"]
    );
    assert_eq!(root.cells()[0].kind, adam_rs::CellKind::Source);
    assert_eq!(root.cells()[1].kind, adam_rs::CellKind::Out);
    let export = compiled.template(&caller, "call").unwrap().clone();
    drop(compiled);
    let mut next = ModuleSet::new();
    next.insert_template("next".parse().unwrap(), "copy", export)
        .unwrap();
    let next = ModuleCompiler::new(TypeRegistry::new(), OpLookup::new())
        .compile(next)
        .unwrap();
    assert!(
        next.sources()
            .iter()
            .any(|source| source.file_name() == "library.adm2")
    );
    let template = next.template(&"next".parse().unwrap(), "copy").unwrap();
    let (mut sheet, a, b) = destination();
    let instance = template
        .instantiate(&mut sheet, &[("b", b), ("a", a)])
        .unwrap();
    drop(next);
    drop(instance);
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 7);
    assert_eq!(sheet.cell_names.len(), 2);
    assert!(
        sheet
            .sources
            .iter()
            .any(|s| s.file_name() == "library.adm2")
    );
    assert!(sheet.sources.iter().any(|s| s.file_name() == "caller.adm2"));
    assert!(
        sheet.cell_origins[&a]
            .iter()
            .any(|o| o.label.location.is_some())
    );
    assert!(
        sheet
            .instance_origins
            .iter()
            .flat_map(|i| &i.occurrences)
            .any(|o| o
                .authored_items()
                .iter()
                .any(|i| i.label.location.is_some()))
    );
}

/// Lowers private policies, sources, outputs, conditionals and branch uses in authored order.
#[test]
fn compiled_bodies_switch_branches_and_keep_instances_fresh() {
    let path: ModulePath = "body".parse().unwrap();
    let mut modules = ModuleSet::new();
    modules
        .insert_source(
            path.clone(),
            "body.adm2",
            "sheet helper(a: i32, b: i32) { relationship { b := a; } }
         sheet all(a: i32, b: i32, flag: bool) {
           cell private: i32 = 4 filter _ - _ + 4 require { private >= 0; @positive private > 0; };
           source limit: i32 = 10 filter 0..=20;
           cell empty: (i32, [f64]);
           out result := private + limit;
           conditional flag {
             true => { relationship helper(a: result_input, b: b); }
             _ => { relationship { b := a + 2; } }
           }
         }",
        )
        .unwrap();
    // A genuinely invalid branch remains an error even when it would be inactive.
    assert!(
        ModuleCompiler::new(TypeRegistry::new(), OpLookup::new())
            .compile(modules)
            .is_err()
    );
    let mut modules = ModuleSet::new();
    modules
        .insert_source(
            path.clone(),
            "body.adm2",
            "sheet helper(a: i32, b: i32) { relationship { b := a; } }
         sheet all(a: i32, b: i32, flag: bool) {
           cell private: i32 = 4 filter _ - _ + 4 require { private >= 0; @positive private > 0; };
           source limit: i32 = 10 filter 0..=20;
           cell empty: (i32, [f64]);
           out result := private + limit;
           conditional flag {
             true => { relationship helper(a: a, b: b); }
             _ => { relationship { b := a + 2; } }
           }
         }",
        )
        .unwrap();
    let compiled = ModuleCompiler::new(TypeRegistry::new(), OpLookup::new())
        .compile(modules)
        .unwrap();
    let template = compiled.template(&path, "all").unwrap();
    let (mut sheet, a, b) = destination();
    let flag = sheet.add_cell(true);
    sheet
        .register_cell(
            "flag",
            flag,
            TypeShape::Named(TypeId::of::<bool>()),
            &TypeRegistry::new(),
        )
        .unwrap();
    let first = template
        .instantiate(&mut sheet, &[("a", a), ("b", b), ("flag", flag)])
        .unwrap();
    let private = first.cells()[3];
    let result = first.cells()[6];
    let mut other = destination().0;
    let a2 = other.cell_names["a"].0;
    let b2 = other.cell_names["b"].0;
    let flag2 = other.add_cell(false);
    other
        .register_cell(
            "flag",
            flag2,
            TypeShape::Named(TypeId::of::<bool>()),
            &TypeRegistry::new(),
        )
        .unwrap();
    let second = template
        .instantiate(&mut other, &[("a", a2), ("b", b2), ("flag", flag2)])
        .unwrap();
    drop(compiled);
    drop((first, second));
    sheet.propagate().unwrap();
    other.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 3);
    assert_eq!(*sheet.read::<i32>(private).unwrap(), 4);
    assert_eq!(*sheet.read::<i32>(result).unwrap(), 14);
    let requirements = sheet.cell_requirements(private).unwrap();
    assert_eq!(requirements.len(), 2);
    assert!(!sheet.requirement_names.contains_key(&requirements[0]));
    assert_eq!(sheet.requirement_names[&requirements[1]], "positive");
    assert_eq!(*other.read::<i32>(b2).unwrap(), 5);
    sheet.write(flag, false).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 5);
    assert_eq!(sheet.cell_names.len(), 3);
    assert_eq!(
        sheet.requirement_names.values().collect::<Vec<_>>(),
        [&"positive".to_owned()]
    );
}

/// Keeps host defaults and callbacks dormant until construction or propagation.
#[test]
fn compiled_factories_do_not_execute_and_preserve_host_shapes() {
    /// Supplies a counted arbitrary registered leaf.
    #[derive(Clone, Debug, PartialEq)]
    struct Host(i32);
    thread_local! {
        static DEFAULTS: Cell<usize> = const { Cell::new(0) };
        static CALLS: Cell<usize> = const { Cell::new(0) };
    }
    impl Default for Host {
        /// Counts actual default construction.
        fn default() -> Self {
            DEFAULTS.with(|c| c.set(c.get() + 1));
            Self(0)
        }
    }
    let mut types = TypeRegistry::new();
    types.register::<Host>("Host");
    let mut lookup = OpLookup::new();
    lookup.push_scope(|name, body, arity, _| {
        if arity != 0 {
            return Ok(false);
        }
        match name {
            "probe" => body.op0(|| {
                CALLS.with(|c| c.set(c.get() + 1));
                1i32
            }),
            "host_value" => body.op0(|| {
                CALLS.with(|c| c.set(c.get() + 1));
                Host(7)
            }),
            _ => return Ok(false),
        }
        Ok(true)
    });
    let path: ModulePath = "host".parse().unwrap();
    let mut modules = ModuleSet::new();
    modules
        .insert_source(
            path.clone(),
            "host.adm2",
            "sheet all() {
          cell host: Host = host_value; cell nested: (Host, [Host]);
          cell copied: Host;
          relationship { copied := host; }
          cell a: i32 = probe filter _ - _ + probe require { a >= probe; };
          conditional probe { 1 => { relationship { a := probe; } } _ => {} }
        }",
        )
        .unwrap();
    let compiled = ModuleCompiler::new(types, lookup).compile(modules).unwrap();
    CALLS.with(|c| assert_eq!(c.get(), 0));
    DEFAULTS.with(|c| assert_eq!(c.get(), 0));
    let template = compiled.template(&path, "all").unwrap().clone();
    let compiler = compiled.compiler();
    drop(compiled);
    let mut again = ModuleSet::new();
    again
        .insert_source(
            path.clone(),
            "again.adm2",
            "sheet again() { cell value: Host; out probe_value := probe; }",
        )
        .unwrap();
    let again = compiler.compile(again).unwrap();
    CALLS.with(|c| assert_eq!(c.get(), 0));
    DEFAULTS.with(|c| assert_eq!(c.get(), 0));
    let mut sheet = ParsedSheet::new();
    let instance = template.instantiate(&mut sheet, &[]).unwrap();
    let copied = instance.cells()[2];
    drop((instance, template));
    DEFAULTS.with(|c| assert_eq!(c.get(), 2));
    assert_eq!(sheet.read::<Host>(copied).unwrap(), &Host(0));
    CALLS.with(|c| assert!(c.get() > 0));
    sheet.propagate().unwrap();
    assert_eq!(sheet.read::<Host>(copied).unwrap(), &Host(7));
    let mut destination = ParsedSheet::new();
    again
        .template(&path, "again")
        .unwrap()
        .instantiate(&mut destination, &[])
        .unwrap();
    drop(again);
    destination.propagate().unwrap();
    DEFAULTS.with(|c| assert_eq!(c.get(), 3));
}

/// Preserves native exports and whole-method pruning without changing bound identities.
#[test]
fn compiled_native_and_parsed_definitions_share_exports_and_source_masks() {
    let mut native = CompositionBuilder::new(TypeRegistry::new());
    let a = native.parameter("a", int()).unwrap();
    let b = native.parameter("b", int()).unwrap();
    let method = native
        .method(std::slice::from_ref(&a), std::slice::from_ref(&b), || {
            let mut s = cel_runtime::DynSegment::new::<()>();
            s.push_arg::<i32>(0);
            Ok(s)
        })
        .unwrap();
    native.relationship(vec![method]).unwrap();
    let path: ModulePath = "mixed".parse().unwrap();
    let mut modules = ModuleSet::new();
    modules
        .insert_template(path.clone(), "native", native.finish().unwrap())
        .unwrap();
    modules
        .insert_source(
            path.clone(),
            "mixed.adm2",
            "sheet caller(a: i32, b: i32) { relationship native(b: b, a: a); }
         sheet tuple(a: i32, b: i32, c: i32) {
           relationship { (a,b) := (c,c); c := a + b; }
         }",
        )
        .unwrap();
    let compiled = ModuleCompiler::new(TypeRegistry::new(), OpLookup::new())
        .compile(modules)
        .unwrap();
    let mut sheet = ParsedSheet::new();
    let a = sheet.add_source(3i32);
    let b = sheet.add_cell(9i32);
    let c = sheet.add_cell(0i32);
    let types = TypeRegistry::new();
    for (name, id) in [("a", a), ("b", b), ("c", c)] {
        sheet.register_cell(name, id, int(), &types).unwrap();
    }
    compiled
        .template(&path, "tuple")
        .unwrap()
        .instantiate(&mut sheet, &[("a", a), ("b", b), ("c", c)])
        .unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 9);
    assert_eq!(*sheet.read::<i32>(c).unwrap(), 12);
    assert!(sheet.method_origins.values().all(|o| o.authored_index == 1));
    compiled
        .template(&path, "caller")
        .unwrap()
        .instantiate(&mut sheet, &[("a", a), ("b", b)])
        .unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 3);
}

/// Distinguishes one-field destructuring, whole tuples and multi-field destructuring.
#[test]
fn compiled_methods_keep_explicit_tuple_patterns_and_recursive_values() {
    let path: ModulePath = "patterns".parse().unwrap();
    let mut modules = ModuleSet::new();
    modules
        .insert_source(
            path.clone(),
            "patterns.adm2",
            "sheet patterns(b: i32) {
               cell scalar: i32; cell whole: (i32,); cell x: i32; cell y: i32;
               cell nested: (i32, [f64]) = (8, [2.0, 3.0]);
               out nested_copy := nested;
               relationship { (scalar,) := (b,); }
               relationship { whole := (b,); }
               relationship { (x,y) := (b, b + 1); }
             }",
        )
        .unwrap();
    let compiled = ModuleCompiler::new(TypeRegistry::new(), OpLookup::new())
        .compile(modules)
        .unwrap();
    let template = compiled.template(&path, "patterns").unwrap();
    let mut sheet = ParsedSheet::new();
    let b = sheet.add_source(19i32);
    sheet
        .register_cell("b", b, int(), &TypeRegistry::new())
        .unwrap();
    let instance = template.instantiate(&mut sheet, &[("b", b)]).unwrap();
    let ids = instance.cells().to_vec();
    drop((instance, compiled));
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(ids[1]).unwrap(), 19);
    assert_eq!(
        sheet
            .read_compound::<cel_runtime::Cons<cel_runtime::Leaf<i32>, cel_runtime::Nil>>(ids[2])
            .unwrap()
            .1,
        19
    );
    assert_eq!(*sheet.read::<i32>(ids[3]).unwrap(), 19);
    assert_eq!(*sheet.read::<i32>(ids[4]).unwrap(), 20);
    assert_eq!(sheet.cell_shape(ids[5]), sheet.cell_shape(ids[6]));
    /// Witnesses the complete nested output shape.
    type Nested = cel_runtime::Cons<
        cel_runtime::Leaf<i32>,
        cel_runtime::Cons<cel_runtime::ArrayOf<cel_runtime::Leaf<f64>>, cel_runtime::Nil>,
    >;
    let nested = sheet.read_compound::<Nested>(ids[6]).unwrap();
    assert_eq!(nested.1, 8);
    assert_eq!(nested.0.1.as_slice(), [2.0, 3.0]);
    assert_eq!(sheet.cell_names.len(), 1);
}

/// Retains typed construction failures and exact sources after every factory owner drops.
#[test]
fn compiled_failures_retain_typed_causes_and_invocation_sources() {
    let path: ModulePath = "failure".parse().unwrap();
    let mut lookup = OpLookup::new();
    lookup.push_scope(|name, body, arity, _| {
        if name != "fail" || arity != 0 {
            return Ok(false);
        }
        body.op0r(|| -> anyhow::Result<i32> { anyhow::bail!("sentinel construction failure") });
        Ok(true)
    });
    let mut modules = ModuleSet::new();
    modules
        .insert_source(
            path.clone(),
            "failure.adm2",
            "sheet failer() { cell bad: i32 = fail; } sheet call() { relationship failer(); }",
        )
        .unwrap();
    let compiled = ModuleCompiler::new(TypeRegistry::new(), lookup)
        .compile(modules)
        .unwrap();
    let template = compiled.template(&path, "call").unwrap().clone();
    drop(compiled);
    let mut destination = ParsedSheet::new();
    let error = template.instantiate(&mut destination, &[]).unwrap_err();
    drop((template, destination));
    let cause = std::error::Error::source(&error).expect("native error must not become a string");
    assert!(cause.downcast_ref::<adam_rs::TemplateError>().is_some());
    assert!(
        error
            .labels
            .iter()
            .filter_map(|l| l.location.as_ref())
            .any(|l| l.source.file_name() == "failure.adm2" && l.source.text().contains("failer"))
    );
    assert!(
        error
            .labels
            .iter()
            .any(|label| label.label == "component use")
    );
}

/// Keeps caller priorities, initial identities, filters and requirements unchanged by binding.
#[test]
fn compiled_binding_does_not_write_or_replace_caller_policies() {
    let path: ModulePath = "copy".parse().unwrap();
    let mut modules = ModuleSet::new();
    modules
        .insert_source(
            path.clone(),
            "copy.adm2",
            "sheet copy(a: i32, b: i32) { relationship { a := b; b := a; } }",
        )
        .unwrap();
    let compiled = ModuleCompiler::new(TypeRegistry::new(), OpLookup::new())
        .compile(modules)
        .unwrap();
    let (mut sheet, a, b) = destination();
    sheet
        .add_filter(
            a,
            adam_rs::Filter::from_fn_0(|value: &i32| Ok((*value).min(10))),
        )
        .unwrap();
    let requirement = sheet
        .add_requirement(
            a,
            adam_rs::Requirement::from_fn_1(a, |value: &i32| Ok(*value >= 0)),
        )
        .unwrap();
    sheet.write(b, 8).unwrap();
    let instance = compiled
        .template(&path, "copy")
        .unwrap()
        .instantiate(&mut sheet, &[("a", a), ("b", b)])
        .unwrap();
    assert_eq!(&instance.cells()[..2], [a, b]);
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 3);
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 8);
    assert_eq!(sheet.cell_requirements(a).unwrap(), [requirement]);
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 8);
    sheet.write(a, 30).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(a).unwrap(), 10);
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 10);
    assert!(sheet.cell_requirements_valid(a));
}

/// Evaluates overridden literal operations only at construction, not compile or propagation.
#[test]
fn compiled_keys_keep_literal_operation_semantics_without_eager_execution() {
    thread_local! {
        static NEGATIONS: Cell<usize> = const { Cell::new(0) };
    }
    let mut lookup = OpLookup::new();
    lookup.push_scope(|name, body, arity, _| {
        if name != "-" || arity != 1 || body.peek_output_type_id() != Some(TypeId::of::<i32>()) {
            return Ok(false);
        }
        body.op1r(|value: i32| -> anyhow::Result<i32> {
            NEGATIONS.with(|count| count.set(count.get() + 1));
            value
                .checked_neg()
                .ok_or_else(|| anyhow::anyhow!("overflow"))
        })?;
        Ok(true)
    });
    let path: ModulePath = "keys".parse().unwrap();
    let mut modules = ModuleSet::new();
    modules
        .insert_source(
            path.clone(),
            "keys.adm2",
            "sheet choose(a: i32, b: i32, key: i32) {
          conditional key { -1 => { relationship { b := a + 1; } }
                            _ => { relationship { b := a + 2; } } }
         }",
        )
        .unwrap();
    let compiled = ModuleCompiler::new(TypeRegistry::new(), lookup)
        .compile(modules)
        .unwrap();
    NEGATIONS.with(|count| assert_eq!(count.get(), 0));
    let (mut sheet, a, b) = destination();
    let key = sheet.add_source(-1i32);
    sheet
        .register_cell("key", key, int(), &TypeRegistry::new())
        .unwrap();
    compiled
        .template(&path, "choose")
        .unwrap()
        .instantiate(&mut sheet, &[("a", a), ("b", b), ("key", key)])
        .unwrap();
    NEGATIONS.with(|count| assert_eq!(count.get(), 1));
    drop(compiled);
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 4);
    sheet.write(key, 0).unwrap();
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(b).unwrap(), 5);
    NEGATIONS.with(|count| assert_eq!(count.get(), 1));
}

/// Rejects source-emptied inactive branches while retaining all genuine file owners.
#[test]
fn compiled_invalid_inactive_branch_owns_transitive_sources() {
    let mut modules = ModuleSet::new();
    modules
        .insert_source(
            "leaf".parse().unwrap(),
            "leaf.adm2",
            "sheet copy(a: i32, b: i32) { relationship { a := b; b := a; } }",
        )
        .unwrap();
    modules
        .insert_source(
            "middle".parse().unwrap(),
            "middle.adm2",
            "use leaf::copy; sheet wrapper(a: i32, b: i32) { relationship copy(a: a, b: b); }",
        )
        .unwrap();
    modules
        .insert_source(
            "root".parse().unwrap(),
            "root.adm2",
            "use middle::wrapper; sheet root {
          source a: i32; source b: i32;
          conditional false { true => { relationship wrapper(a: a, b: b); } }
         }",
        )
        .unwrap();
    let errors = match ModuleCompiler::new(TypeRegistry::new(), OpLookup::new()).compile(modules) {
        Ok(_) => panic!("inactive source-empty use must fail"),
        Err(errors) => errors,
    };
    for file in ["leaf.adm2", "middle.adm2", "root.adm2"] {
        assert!(
            errors
                .iter()
                .flat_map(|error| &error.labels)
                .filter_map(|label| label.location.as_ref())
                .any(|location| location.source.file_name() == file)
        );
    }
}

/// Keeps emitted execution owners fresh across instances and alive after compiler drop.
#[test]
fn compiled_instances_own_distinct_execution_resources() {
    thread_local! {
        static OWNERS: std::cell::RefCell<Vec<std::rc::Rc<()>>> = const { std::cell::RefCell::new(Vec::new()) };
        static EMISSIONS: Cell<usize> = const { Cell::new(0) };
    }
    let mut lookup = OpLookup::new();
    lookup.push_scope(|name, body, arity, _| {
        if name != "owner" || arity != 0 {
            return Ok(false);
        }
        EMISSIONS.with(|count| count.set(count.get() + 1));
        let owner = std::rc::Rc::new(());
        body.op0(move || {
            OWNERS.with(|owners| owners.borrow_mut().push(owner.clone()));
            7i32
        });
        Ok(true)
    });
    let mut modules = ModuleSet::new();
    let path: ModulePath = "owners".parse().unwrap();
    modules
        .insert_source(
            path.clone(),
            "owners.adm2",
            "sheet owners() { out value := owner; }",
        )
        .unwrap();
    let compiled = ModuleCompiler::new(TypeRegistry::new(), lookup)
        .compile(modules)
        .unwrap();
    let template = compiled.template(&path, "owners").unwrap();
    OWNERS.with(|owners| assert!(owners.borrow().is_empty()));
    let mut a = ParsedSheet::new();
    let mut b = ParsedSheet::new();
    template.instantiate(&mut a, &[]).unwrap();
    template.instantiate(&mut b, &[]).unwrap();
    let emissions = EMISSIONS.with(Cell::get);
    drop(compiled);
    a.propagate().unwrap();
    b.propagate().unwrap();
    EMISSIONS.with(|count| assert_eq!(count.get(), emissions));
    OWNERS.with(|owners| {
        let owners = owners.borrow();
        assert_eq!(owners.len(), 2);
        assert!(!std::rc::Rc::ptr_eq(&owners[0], &owners[1]));
    });
}

/// Preserves first-use order, candidate slot zero and exact range-policy results.
#[test]
fn compiled_positional_reads_and_candidate_filters_keep_checked_order() {
    let path: ModulePath = "order".parse().unwrap();
    let mut modules = ModuleSet::new();
    modules
        .insert_source(
            path.clone(),
            "order.adm2",
            "sheet order(a: i32, b: i32, c: i32) {
           source empty: i32;
           cell value: i32 = 15 filter b - b + _ % (b + a);
           cell ranged: i32 = 10 filter b + (_ - _) ..= a;
           out summary := value + ranged;
           relationship { c := b * 10 + a; }
         }",
        )
        .unwrap();
    let compiled = ModuleCompiler::new(TypeRegistry::new(), OpLookup::new())
        .compile(modules)
        .unwrap();
    let mut sheet = ParsedSheet::new();
    let a = sheet.add_source(7i32);
    let b = sheet.add_source(3i32);
    let c = sheet.add_cell(0i32);
    for (name, id) in [("a", a), ("b", b), ("c", c)] {
        sheet
            .register_cell(name, id, int(), &TypeRegistry::new())
            .unwrap();
    }
    let instance = compiled
        .template(&path, "order")
        .unwrap()
        .instantiate(&mut sheet, &[("a", a), ("b", b), ("c", c)])
        .unwrap();
    let cells = instance.cells().to_vec();
    drop((instance, compiled));
    sheet.propagate().unwrap();
    assert_eq!(*sheet.read::<i32>(c).unwrap(), 37);
    assert_eq!(*sheet.read::<i32>(cells[3]).unwrap(), 0);
    assert_eq!(sheet.cell_kind(cells[3]), Some(adam_rs::CellKind::Source));
    assert_eq!(*sheet.read::<i32>(cells[4]).unwrap(), 5);
    assert_eq!(*sheet.read::<i32>(cells[5]).unwrap(), 7);
    assert_eq!(*sheet.read::<i32>(cells[6]).unwrap(), 12);
}

/// Retains authored method sites and original runtime errors after successful construction.
#[test]
fn compiled_runtime_failure_keeps_native_error_and_method_metadata() {
    let mut modules = ModuleSet::new();
    let path: ModulePath = "runtime".parse().unwrap();
    modules
        .insert_source(
            path.clone(),
            "runtime.adm2",
            "sheet fail(a: i32, b: i32) { relationship { b := a / 0; } }",
        )
        .unwrap();
    let compiled = ModuleCompiler::new(TypeRegistry::new(), OpLookup::new())
        .compile(modules)
        .unwrap();
    let (mut sheet, a, b) = destination();
    compiled
        .template(&path, "fail")
        .unwrap()
        .instantiate(&mut sheet, &[("a", a), ("b", b)])
        .unwrap();
    drop(compiled);
    let error = sheet.propagate().unwrap_err();
    assert!(matches!(error, adam_rs::Error::MethodFailed { .. }));
    assert!(std::error::Error::source(&error).is_some());
    let origin = sheet.method_origins.values().next().unwrap();
    let occurrence = sheet
        .instance_origins
        .iter()
        .flat_map(|instance| &instance.occurrences)
        .find(|occurrence| occurrence.id == origin.occurrence)
        .unwrap();
    let method = occurrence
        .authored_items()
        .iter()
        .find(|item| {
            item.role == adam_lang::AuthoredRole::Method
                && item.relationship.as_ref() == Some(&origin.relationship)
                && item.method_index == Some(origin.authored_index)
        })
        .unwrap();
    assert_eq!(
        method
            .expression
            .as_ref()
            .unwrap()
            .location
            .as_ref()
            .unwrap()
            .source
            .file_name(),
        "runtime.adm2"
    );
}
