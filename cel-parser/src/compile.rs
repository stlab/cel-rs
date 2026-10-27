//! Compiles retained syntax directly through the parser's runtime emission context.
//!
//! [`compile_expr`] borrows compile-time environments and creates a fresh execution owner.
//! Its external identifier emitter can install full runtime descriptors on the supplied segment.

use cel_runtime::{DynSegment, ValueType};

use crate::{
    DynSegmentContext, Expr, ExprSpan, Literal, LogicalOp, OpLookup, ParseError, ParserContext,
    Result, SourceSpan, TypeResolver,
};

/// Compiles retained syntax into an independently owned executable segment.
///
/// Borrows `lookup` and `types` only during compilation. `resolve` emits an external
/// identifier and returns `true`, or leaves the segment unchanged and returns `false`
/// to continue normal lookup. An emitted value must carry its complete runtime
/// descriptor, including tuple, array, record, or callable shape.
///
/// Noncapturing closure bodies resolve only their own formals and persistent library
/// names; they cannot consult `resolve` or transient lookup scopes. Each compilation
/// creates fresh branch and closure execution resources. Neither syntax nor environments
/// need to outlive the returned segment.
///
/// # Errors
/// Returns a spanned error for unresolved names, incompatible operands, invalid annotations,
/// or external emitter failures. Partially emitted resources remain safely discardable.
///
/// - Complexity: O(n) traversal time and space, plus type resolution, linear formal-name
///   lookup and operation dispatch costs for the n syntax nodes.
///
/// # Examples
/// ```
/// use cel_parser::{AstContext, OpLookup, Parser, builtin_type_resolver, compile_expr};
/// let expr = Parser::<AstContext>::new(OpLookup::new())
///     .parse_str_ast("false && (1i32 / 0i32 == 0i32)").unwrap();
/// let types = builtin_type_resolver();
/// let mut segment = compile_expr(&expr, &OpLookup::new(), types.as_ref(),
///     &mut |_, _, _| Ok(false)).unwrap();
/// drop(expr);
/// assert!(!segment.call0::<bool>().unwrap());
/// ```
pub fn compile_expr<F>(
    expr: &Expr,
    lookup: &OpLookup,
    types: &dyn TypeResolver,
    resolve: &mut F,
) -> Result<DynSegment>
where
    F: FnMut(&str, &mut DynSegment, SourceSpan) -> anyhow::Result<bool>,
{
    let mut context = DynSegmentContext::new_context();
    emit(expr, &mut context, lookup, types, resolve, None)?;
    Ok(context.into_inner())
}

/// Emits one syntax tree in source evaluation order, isolating names in closure bodies.
///
/// `locals == None` selects the external environment; `Some` selects only the current
/// closure's formal bindings and persistent lookup scopes.
///
/// # Errors
/// Returns a spanned semantic or external-emission error.
/// - Complexity: O(n) traversal time and stack space, plus dispatch and annotation resolution.
fn emit<F>(
    expr: &Expr,
    context: &mut DynSegmentContext,
    lookup: &OpLookup,
    types: &dyn TypeResolver,
    resolve: &mut F,
    locals: Option<&[(&str, ValueType)]>,
) -> Result<()>
where
    F: FnMut(&str, &mut DynSegment, SourceSpan) -> anyhow::Result<bool>,
{
    let span = expr.span();
    let error = |message: String| ParseError::new_range(message, span.start, span.end);
    match expr {
        Expr::Literal { value, .. } => push_literal(context, value, span)?,
        Expr::Ident { name, .. } => {
            if let Some(locals) = locals {
                if let Some((index, (_, ty))) = locals
                    .iter()
                    .enumerate()
                    .rev()
                    .find(|(_, (local, _))| *local == name)
                {
                    return ty
                        .push_argument(context, index)
                        .map_err(|e| error(e.to_string()));
                }
            } else if resolve(
                name,
                context,
                SourceSpan::from_proc_macro2_range(span.start, span.end),
            )
            .map_err(|e| error(e.to_string()))?
            {
                return Ok(());
            }
            context.apply_scoped_op(lookup, name, 0, span, false, locals.is_some())?;
        }
        Expr::Op { name, operands, .. } => {
            for operand in operands {
                emit(operand, context, lookup, types, resolve, locals)?;
            }
            context.apply_scoped_op(lookup, name, operands.len(), span, false, locals.is_some())?;
        }
        Expr::Apply { callee, args, .. } => {
            emit(callee, context, lookup, types, resolve, locals)?;
            for arg in args {
                emit(arg, context, lookup, types, resolve, locals)?;
            }
            context.apply_scoped_op(lookup, "()", args.len() + 1, span, false, locals.is_some())?;
        }
        Expr::Member {
            receiver,
            name,
            name_span,
            args,
            ..
        } => {
            emit(receiver, context, lookup, types, resolve, locals)?;
            if let Some(args) = args {
                for arg in args {
                    emit(arg, context, lookup, types, resolve, locals)?;
                }
            }
            context.apply_scoped_op(
                lookup,
                name,
                1 + args.as_ref().map_or(0, Vec::len),
                *name_span,
                true,
                locals.is_some(),
            )?;
        }
        Expr::Struct {
            name,
            name_span,
            fields,
            ..
        } => {
            let ambient = context.current_stack_offset();
            for field in fields {
                emit(&field.value, context, lookup, types, resolve, locals)?;
            }
            context.make_record(
                lookup,
                name,
                *name_span,
                fields
                    .iter()
                    .map(|field| (field.name.clone(), field.name_span))
                    .collect(),
                ambient,
                span.start,
                span.end,
            )?;
        }
        Expr::Tuple { elements, .. } => {
            let ambient = context.current_stack_offset();
            for element in elements {
                emit(element, context, lookup, types, resolve, locals)?;
            }
            context.make_tuple(elements.len(), ambient, span.start, span.end)?;
        }
        Expr::Array {
            elements,
            type_annotation,
            ..
        } => {
            let ambient = context.current_stack_offset();
            for element in elements {
                emit(element, context, lookup, types, resolve, locals)?;
            }
            let element_type = type_annotation
                .as_ref()
                .map(|ty| {
                    crate::ResolvedArrayType::from_resolved_type(ty.resolve(types)?, ty.span())
                        .map(|resolved| resolved.into_element_type())
                })
                .transpose()?;
            context.make_resolved_array(elements.len(), ambient, element_type, span)?;
        }
        Expr::TupleIndex { base, index, .. } => {
            emit(base, context, lookup, types, resolve, locals)?;
            let arity = context
                .peek_tuple_arity()
                .ok_or_else(|| error("'.N' requires a tuple".into()))?;
            if *index >= arity {
                return Err(error(format!(
                    "tuple index `{index}` out of range for tuple of arity {arity}"
                )));
            }
            context.tuple_index(*index, span.start, span.end)?;
        }
        Expr::If {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            emit(cond, context, lookup, types, resolve, locals)?;
            let mut then_context = context.new_fragment();
            emit(
                then_branch,
                &mut then_context,
                lookup,
                types,
                resolve,
                locals,
            )?;
            let else_context = if let Some(branch) = else_branch {
                let mut fragment = context.new_fragment();
                emit(branch, &mut fragment, lookup, types, resolve, locals)?;
                Some(fragment)
            } else {
                None
            };
            context
                .join2(then_context, else_context, span.start, span.end)
                .map_err(|e| error(e.to_string()))?;
        }
        Expr::Logical { op, lhs, rhs, .. } => {
            emit(lhs, context, lookup, types, resolve, locals)?;
            let mut fragment = context.new_fragment();
            emit(rhs, &mut fragment, lookup, types, resolve, locals)?;
            context.apply_logical(
                match op {
                    LogicalOp::And => "&&",
                    LogicalOp::Or => "||",
                },
                fragment,
                span.start,
                span.end,
            )?;
        }
        Expr::Cast {
            expr, type_name, ..
        } => {
            emit(expr, context, lookup, types, resolve, locals)?;
            context.apply_cast(lookup, type_name, span.start, span.end)?;
        }
        Expr::Closure { params, body, .. } => {
            let bindings = params
                .iter()
                .map(|param| {
                    crate::closure_param_type_from_type_expr(&param.type_expr, types)
                        .map(|ty| (param.name.as_str(), ty))
                })
                .collect::<Result<Vec<_>>>()?;
            let mut body_context = DynSegmentContext::new_context();
            emit(
                body,
                &mut body_context,
                lookup,
                types,
                resolve,
                Some(&bindings),
            )?;
            context.push_closure(
                bindings.into_iter().map(|(_, ty)| ty).collect(),
                Vec::new(),
                body_context,
                span.start,
            )?;
        }
    }
    Ok(())
}

/// Copies a retained literal into fresh segment-owned storage.
///
/// # Errors
/// Returns an error if the canonical unit layout cannot be represented.
/// - Complexity: O(n) time and space for an n-byte owned literal; O(1) otherwise.
fn push_literal(context: &mut DynSegmentContext, value: &Literal, span: ExprSpan) -> Result<()> {
    macro_rules! push {
        ($($variant:ident),* $(,)?) => {
            match value {
                $(Literal::$variant(value) => context.push_literal(value.clone(), span.start),)*
                Literal::Unit => context.make_tuple(
                    0, context.current_stack_offset(), span.start, span.end,
                )?,
            }
        };
    }
    push!(
        I8, I16, I32, I64, I128, Isize, U8, U16, U32, U64, U128, Usize, F32, F64, Bool, Char, Str,
        ByteStr, CStr
    );
    Ok(())
}
