//! A first-class, callable CEL value: a compiled body plus its declared signature.
//!
//! See `docs/superpowers/specs/2026-08-21-cel-closures-design.md` for the full design rationale
//! (why `Rc`, why `RefCell`, why no captured environment).
//!
//! # Examples
//!
//! Build a closure that adds two `i32` values, then call it with different arguments:
//!
//! ```rust
//! use cel_runtime::{DynClosure, DynSegment};
//! use std::any::TypeId;
//!
//! // Compile a body that takes two i32 arguments and returns their sum.
//! let mut body = DynSegment::new::<()>();
//! body.push_arg::<i32>(0);
//! body.push_arg::<i32>(1);
//! body.op2(|a: i32, b: i32| a + b).unwrap();
//!
//! // Wrap it as a callable closure with its declared signature.
//! let closure = DynClosure::new(
//!     vec![TypeId::of::<i32>(), TypeId::of::<i32>()],
//!     TypeId::of::<i32>(),
//!     body,
//! );
//!
//! // Call the closure multiple times with different arguments.
//! let (a, b) = (2i32, 3i32);
//! assert_eq!(closure.call::<i32>(&[&a, &b]).unwrap(), 5);
//!
//! let (c, d) = (10i32, 20i32);
//! assert_eq!(closure.call::<i32>(&[&c, &d]).unwrap(), 30);
//! ```

use std::any::{Any, TypeId};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::ValueType;
use crate::dyn_segment::DynSegment;

/// An exact compile-time callable shape, including nested aggregate and callable results.
#[derive(Clone, Debug)]
pub(crate) struct ClosureSignature {
    pub(crate) parameters: Vec<ValueType>,
    pub(crate) result: ValueType,
}

impl ClosureSignature {
    /// Converts known recursive metadata without changing canonical storage.
    ///
    /// # Errors
    /// Returns an error for missing or mismatched leaf factories, unknown nested
    /// callable signatures, or an unrepresentable canonical tuple layout.
    /// - Complexity: O(n) traversal time and output space for n signature nodes,
    ///   excluding leaf factory costs.
    fn complete(
        &self,
        leaf: &mut impl FnMut(TypeId) -> Option<crate::value_type::ValueType>,
    ) -> anyhow::Result<crate::value_type::ValueType> {
        let parameters = self
            .parameters
            .iter()
            .map(|parameter| enrich_value_type(parameter, leaf))
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(crate::value_type::ValueType::callable(
            parameters,
            enrich_value_type(&self.result, leaf)?,
        ))
    }
}

/// Resolves one concrete registered leaf through a previously monomorphized factory.
///
/// # Errors
/// Returns an error if the factory is missing, structural, or describes another
/// concrete type or layout. A closure marker without a signature is rejected.
fn complete_leaf(
    id: TypeId,
    size: usize,
    align: usize,
    leaf: &mut impl FnMut(TypeId) -> Option<crate::value_type::ValueType>,
) -> anyhow::Result<crate::value_type::ValueType> {
    anyhow::ensure!(
        id != TypeId::of::<DynClosure>(),
        "closure signature metadata is unavailable"
    );
    let complete =
        leaf(id).ok_or_else(|| anyhow::anyhow!("complete leaf factory is unavailable"))?;
    complete.validate_opaque_leaf(id)?;
    anyhow::ensure!(
        complete.layout().size() == size && complete.layout().align() == align,
        "complete leaf factory layout mismatch"
    );
    Ok(complete)
}

/// Enriches canonical signature leaves through explicitly supplied factories.
///
/// # Errors
/// Returns an error for unavailable signatures/factories or invalid tuple layout.
/// - Complexity: O(n) time and output space for n recursive descriptor nodes.
fn enrich_value_type(
    value: &ValueType,
    leaf: &mut impl FnMut(TypeId) -> Option<crate::value_type::ValueType>,
) -> anyhow::Result<crate::value_type::ValueType> {
    use crate::value_type::{AssociatedType, ValueType as Complete};
    match value.kind() {
        crate::ValueKind::Tuple(fields) => Complete::tuple(
            fields
                .iter()
                .map(|field| {
                    Ok(AssociatedType {
                        offset: 0,
                        value_type: enrich_value_type(&field.value_type, leaf)?,
                    })
                })
                .collect::<anyhow::Result<Vec<_>>>()?,
        ),
        crate::ValueKind::Array(element) => Ok(Complete::array(enrich_value_type(element, leaf)?)),
        crate::ValueKind::Callable { parameters, result } => Ok(Complete::callable(
            parameters
                .iter()
                .map(|parameter| enrich_value_type(parameter, leaf))
                .collect::<anyhow::Result<_>>()?,
            enrich_value_type(result, leaf)?,
        )),
        crate::ValueKind::Leaf(id) => complete_leaf(*id, value.size(), value.align(), leaf),
    }
}

/// Compares complete known signatures without relying on the erased closure marker.
///
/// - Complexity: O(number of recursive type nodes).
#[cfg(test)]
pub(crate) fn same_signature(
    left: Option<&ClosureSignature>,
    right: Option<&ClosureSignature>,
) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            a.parameters.len() == b.parameters.len()
                && a.parameters
                    .iter()
                    .zip(&b.parameters)
                    .all(|(a, b)| a.same_shape(b))
                && a.result.same_shape(&b.result)
        }
        _ => false,
    }
}

/// A monomorphized dispatcher for [`DynClosure::call_boxed`]: runs a closure's body via
/// `DynSegment::call_dyn::<R>` for some concrete `R` the dispatcher itself already knows, boxing
/// the result so the caller (which only knows `R` dynamically, as a `TypeId`) doesn't need to.
pub type CallDynFn = fn(&mut DynSegment, &[&dyn Any]) -> anyhow::Result<Box<dyn Any>>;

/// Invokes an exact construction-bound payload and initializes output on success only.
///
/// Both frame pointers and the payload must match the dispatcher's concrete types.
pub(crate) type FrameDispatch = unsafe fn(*mut (), *const u8, *mut u8) -> anyhow::Result<()>;

/// Owns either a compiler-facing body or a private type-bound frame dispatcher.
enum ClosureBody {
    Interpreted(DynSegment),
    Framed {
        payload: Box<dyn Any>,
        dispatch: FrameDispatch,
    },
}

impl ClosureBody {
    /// Returns the compiler-facing body, rejecting immutable typed facade bodies.
    ///
    /// # Errors
    /// Rejects framed native and prepared bodies.
    fn segment(&mut self) -> anyhow::Result<&mut DynSegment> {
        match self {
            Self::Interpreted(body) => Ok(body),
            Self::Framed { .. } => anyhow::bail!("typed callable has no mutable compiler body"),
        }
    }
}

/// Binds a canonical contiguous argument frame and a raw interpreted result once.
struct FramePlan {
    offsets: Vec<usize>,
    result: crate::container_execution::ResultPlan,
}

/// Shares one body and counts live trusted-binding leases across all carrier aliases.
struct ClosureData {
    param_types: Vec<TypeId>,
    return_type: TypeId,
    body: RefCell<ClosureBody>,
    frame: Option<FramePlan>,
    signature: Option<std::sync::Arc<ClosureSignature>>,
    trusted_bindings: Cell<usize>,
}

/// Prevents arbitrary mutable body dispatch until this independent lease is dropped.
///
/// Holds the shared carrier alive; moving the lease preserves the freeze, and
/// dropping it releases exactly one binding even during unwinding. Carrier clones
/// share the counter but neither acquire nor release leases themselves.
pub(crate) struct TrustedBindingLease {
    closure: DynClosure,
}

impl Drop for TrustedBindingLease {
    /// Decrements the lease count without borrowing the body, before releasing the carrier.
    fn drop(&mut self) {
        let bindings = &self.closure.0.trusted_bindings;
        debug_assert!(bindings.get() > 0);
        bindings.set(bindings.get() - 1);
    }
}

/// `Debug` implementation that formats the closure's signature but deliberately omits `body` from the output
/// (via `finish_non_exhaustive`), since `DynSegment` is not easily debuggable and the signature alone
/// is typically what callers need to see.
impl std::fmt::Debug for ClosureData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClosureData")
            .field("param_types", &self.param_types)
            .field("return_type", &self.return_type)
            .finish_non_exhaustive()
    }
}

/// A first-class, callable CEL value: a compiled body plus its declared parameter/return types.
///
/// Holds no captured environment — only its own parameters resolve inside `body`. Calling it
/// twice with different `args` is exactly as fresh each time as calling any other `DynSegment`.
/// `Rc`-wrapped so `Clone` never requires `DynSegment` itself to implement `Clone` (it doesn't —
/// see the design spec); `RefCell` because callers only ever reach a `DynClosure` through `&self`
/// (matching `adam_rs::Filter`/`Method`'s `Fn`, not `FnMut`, storage), while `DynSegment`'s own
/// call methods need `&mut self`.
#[derive(Clone, Debug)]
pub struct DynClosure(Rc<ClosureData>, Option<Rc<TrustedBindingLease>>);

impl std::fmt::Debug for TrustedBindingLease {
    /// Formats the lease without recursively traversing its retained carrier.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TrustedBindingLease")
    }
}

impl DynClosure {
    /// Shares a construction-bound frame dispatcher without exposing its payload.
    ///
    /// A prepared payload owns its immutable body and all nested trusted leases.
    ///
    /// # Safety
    /// `dispatch` must match `payload` and the facade's exact Rust signature.
    /// On success it must initialize exactly one result; on error or unwind it
    /// must leave output uninitialized. If supplied, metadata must describe that
    /// exact signature.
    /// - Complexity: O(p) parameter metadata storage for p declared parameters.
    pub(crate) unsafe fn from_frame_body(
        payload: Box<dyn Any>,
        dispatch: FrameDispatch,
        signature: Option<ClosureSignature>,
    ) -> Self {
        let param_types = signature
            .as_ref()
            .map(|s| s.parameters.iter().map(ValueType::type_id).collect())
            .unwrap_or_default();
        let return_type = signature
            .as_ref()
            .map(|s| s.result.type_id())
            .unwrap_or(TypeId::of::<()>());
        Self(
            Rc::new(ClosureData {
                param_types,
                return_type,
                body: RefCell::new(ClosureBody::Framed { payload, dispatch }),
                frame: None,
                signature: signature.map(std::sync::Arc::new),
                trusted_bindings: Cell::new(0),
            }),
            None,
        )
    }

    /// Dispatches a complete preparation-validated contiguous borrowed argument frame.
    ///
    /// # Safety
    /// Both frame pointers must match this exact carrier's validated signature.
    /// Output is aligned uninitialized result storage. A compiler-facing interpreted body
    /// must be protected by a trusted lease for the duration of the call.
    /// # Errors
    /// Returns reentrant entry or original body errors without initializing output.
    /// - Complexity: Body-dependent; interpreted bodies set up O(a) pointers.
    pub(crate) unsafe fn call_frame_trusted(
        &self,
        args: *const u8,
        result: *mut u8,
    ) -> anyhow::Result<()> {
        let mut body = self
            .0
            .body
            .try_borrow_mut()
            .map_err(|_| anyhow::anyhow!("closure body is already executing"))?;
        match &mut *body {
            ClosureBody::Framed { payload, dispatch } => {
                // Erasing an Any pointer preserves the boxed concrete data
                // address; construction binds its monomorphized dispatcher.
                let pointer = std::ptr::from_mut(payload.as_mut()).cast::<()>();
                unsafe { dispatch(pointer, args, result) }
            }
            ClosureBody::Interpreted(body) => {
                let frame = self.0.frame.as_ref().expect("validated canonical body");
                let arguments: Vec<_> = frame
                    .offsets
                    .iter()
                    .map(|offset| unsafe { args.add(*offset) })
                    .collect();
                let mut stack = unsafe { body.execute_raw(&arguments)? };
                let mut output = unsafe { frame.result.take(&mut stack) };
                unsafe { output.move_into(result) };
                Ok(())
            }
        }
    }

    /// Returns the complete canonical callable descriptor after validating its current body.
    ///
    /// # Errors
    /// Rejects untyped closures, executing bodies, or bodies changed incompatibly
    /// through an unfrozen mutable dispatcher.
    /// - Complexity: O(n) complete comparisons for n signature/body descriptor nodes.
    ///
    /// # Examples
    /// ```
    /// let mut body = cel_runtime::DynSegment::new::<()>(); body.just(7i32);
    /// let closure = cel_runtime::DynClosure::typed(vec![], body).unwrap();
    /// assert!(matches!(closure.value_type().unwrap().kind(), cel_runtime::ValueKind::Callable { .. }));
    /// ```
    pub fn value_type(&self) -> anyhow::Result<ValueType> {
        self.validate_body_signature()?;
        let signature = self
            .0
            .signature
            .as_ref()
            .expect("validated typed signature");
        Ok(ValueType::callable(
            signature.parameters.clone(),
            signature.result.clone(),
        ))
    }

    /// Wraps `body` as a closure value declaring `param_types` (in order) and `return_type`.
    ///
    /// - Precondition: `body` was compiled expecting exactly `param_types.len()` positional
    ///   arguments (via `push_arg`/`push_argument`), in that order, and
    ///   produces exactly one result of `return_type`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::{DynClosure, DynSegment};
    /// use std::any::TypeId;
    ///
    /// let mut body = DynSegment::new::<()>();
    /// body.push_arg::<i32>(0);
    /// body.push_arg::<i32>(1);
    /// body.op2(|a: i32, b: i32| a + b).unwrap();
    ///
    /// let closure = DynClosure::new(
    ///     vec![TypeId::of::<i32>(), TypeId::of::<i32>()],
    ///     TypeId::of::<i32>(),
    ///     body,
    /// );
    /// assert_eq!(closure.param_types().len(), 2);
    /// ```
    #[must_use]
    pub fn new(param_types: Vec<TypeId>, return_type: TypeId, body: DynSegment) -> Self {
        DynClosure(
            Rc::new(ClosureData {
                param_types,
                return_type,
                body: RefCell::new(ClosureBody::Interpreted(body)),
                frame: None,
                signature: None,
                trusted_bindings: Cell::new(0),
            }),
            None,
        )
    }

    /// Wraps a checked CEL body while retaining its full callable shape.
    ///
    /// # Errors
    /// Returns an error if the body does not produce exactly one value, requires preloaded
    /// arguments or a nonzero ambient stack origin, reads an undeclared or differently shaped
    /// argument, or reads a tuple whose leaves lack clone adapters. A zero-origin fragment is
    /// valid because its operations can access only its local operands and declared arguments.
    ///
    /// - Complexity: O(number of parameters plus recursive nodes in body argument reads/result).
    ///
    /// # Examples
    /// ```
    /// use cel_runtime::{DynClosure, DynSegment, ValueType};
    /// let mut body = DynSegment::new::<()>();
    /// body.just(42i32);
    /// let closure = DynClosure::typed(vec![], body).unwrap();
    /// assert_eq!(closure.call::<i32>(&[]).unwrap(), 42);
    /// ```
    pub fn typed(parameters: Vec<ValueType>, body: DynSegment) -> anyhow::Result<Self> {
        let result = Self::validate_body(&parameters, &body)?;
        let tuple = ValueType::tuple(
            parameters
                .iter()
                .map(|value_type| crate::AssociatedType {
                    offset: 0,
                    value_type: value_type.clone(),
                })
                .collect(),
        )?;
        let frame = FramePlan {
            offsets: tuple
                .tuple_elements()
                .unwrap()
                .iter()
                .map(|f| f.offset)
                .collect(),
            result: crate::container_execution::ResultPlan::new(0, result.clone())?,
        };
        let param_types = parameters.iter().map(ValueType::type_id).collect();
        Ok(Self(
            Rc::new(ClosureData {
                param_types,
                return_type: result.type_id(),
                signature: Some(std::sync::Arc::new(ClosureSignature { parameters, result })),
                body: RefCell::new(ClosureBody::Interpreted(body)),
                frame: Some(frame),
                trusted_bindings: Cell::new(0),
            }),
            None,
        ))
    }

    /// Returns a carrier whose clones retain a trusted freeze for their own lifetime.
    ///
    /// # Errors
    /// Rejects an executing body or an unrepresentable additional lease.
    pub(crate) fn bind_canonical(mut self) -> anyhow::Result<Self> {
        if self.1.is_none() {
            self.1 = Some(Rc::new(self.freeze_trusted_binding()?));
        }
        Ok(self)
    }

    /// Freezes arbitrary mutable body access across all aliases until the returned lease drops.
    ///
    /// This does not validate the signature; preparation must do so before binding.
    /// Existing nonmutating execution paths remain available while leased.
    ///
    /// # Errors
    /// Rejects a mutably borrowed body (including an active mutable dispatcher)
    /// or an unrepresentable additional lease count without changing the counter.
    pub(crate) fn freeze_trusted_binding(&self) -> anyhow::Result<TrustedBindingLease> {
        let _body = self
            .0
            .body
            .try_borrow()
            .map_err(|_| anyhow::anyhow!("closure body is already executing"))?;
        let count = self
            .0
            .trusted_bindings
            .get()
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("too many trusted bindings"))?;
        let lease = TrustedBindingLease {
            closure: self.clone(),
        };
        self.0.trusted_bindings.set(count);
        Ok(lease)
    }

    /// Validates independent execution and declared argument reads, returning the actual result.
    ///
    /// # Errors
    /// Rejects non-single results, dependent bodies, undeclared or mismatched reads,
    /// and tuple reads without clone adapters.
    /// - Complexity: O(recursive nodes in body argument reads and result).
    fn validate_body(parameters: &[ValueType], body: &DynSegment) -> anyhow::Result<ValueType> {
        anyhow::ensure!(
            body.stack_ids.len() == 1,
            "closure body must produce exactly one value"
        );
        body.validate_independent_body()?;
        for (index, read_type) in &body.argument_reads {
            anyhow::ensure!(
                parameters
                    .get(*index)
                    .is_some_and(|expected| expected.same_shape(read_type)),
                "closure body argument {index} differs from its declared signature"
            );
            read_type.validate_clone()?;
        }
        Ok(body.peek_stack_infos(1)[0].value_type.clone())
    }

    /// Checks the current mutable body against its retained typed signature during preparation.
    ///
    /// An external mutable dispatcher may have replaced the body without updating its metadata.
    ///
    /// # Errors
    /// Rejects unavailable signatures, an executing body, invalid independent bodies
    /// or argument reads, and a result differing from the retained signature.
    /// - Complexity: O(recursive nodes in body argument reads and result).
    pub(crate) fn validate_body_signature(&self) -> anyhow::Result<()> {
        let signature = self
            .0
            .signature
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("closure signature metadata is unavailable"))?;
        let body = self
            .0
            .body
            .try_borrow()
            .map_err(|_| anyhow::anyhow!("closure body is already executing"))?;
        let ClosureBody::Interpreted(body) = &*body else {
            return Ok(());
        };
        let result = Self::validate_body(&signature.parameters, body)?;
        anyhow::ensure!(
            signature.result.same_shape(&result),
            "closure body result differs from its declared signature"
        );
        Ok(())
    }

    /// Returns the metadata retained for a typed CEL closure.
    pub(crate) fn signature(&self) -> Option<std::sync::Arc<ClosureSignature>> {
        self.0.signature.clone()
    }

    /// Enriches a complete canonical signature with explicitly supplied leaf factories.
    ///
    /// `leaf` looks up descriptors built while each concrete Rust type was
    /// known; a `TypeId` never synthesizes a Rust monomorphization.
    /// Factory capabilities remain independent of identity. This changes metadata
    /// only; [`Self::value_type`] needs no factories for the existing complete signature.
    ///
    /// # Errors
    ///
    /// Returns an error for an untyped closure (including [`Self::new`]), an
    /// unknown nested callable signature, a missing or mismatched concrete leaf
    /// factory, or an unrepresentable canonical tuple layout.
    ///
    /// - Complexity: O(n) traversal time and output space for n signature nodes,
    ///   excluding leaf factory costs.
    ///
    /// # Examples
    ///
    /// ```
    /// use cel_runtime::{DynClosure, DynSegment, value_type::{ValueKind, ValueType}};
    /// use std::any::TypeId;
    /// let mut body = DynSegment::new::<()>();
    /// body.just(42i32);
    /// let closure = DynClosure::typed(vec![], body).unwrap();
    /// let complete = closure.complete_value_type(|id| {
    ///     (id == TypeId::of::<i32>()).then(ValueType::cloneable_leaf::<i32>)
    /// }).unwrap();
    /// assert!(matches!(complete.kind(), ValueKind::Callable { .. }));
    /// ```
    pub fn complete_value_type(
        &self,
        mut leaf: impl FnMut(TypeId) -> Option<crate::value_type::ValueType>,
    ) -> anyhow::Result<crate::value_type::ValueType> {
        self.0
            .signature
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("closure signature metadata is unavailable"))?
            .complete(&mut leaf)
    }

    /// Evaluates a previously checked borrowed raw argument frame into an independent stack.
    ///
    /// # Safety
    /// Every pointer must reference a live value matching `expected.parameters`, in order,
    /// until execution returns. No pointer may be retained by an operation.
    ///
    /// # Errors
    /// Returns signature mismatch, reentrant body borrow, or body evaluation errors.
    /// - Complexity: body-dependent.
    #[cfg(test)]
    pub(crate) unsafe fn call_raw(
        &self,
        expected: &ClosureSignature,
        arguments: &[*const u8],
    ) -> anyhow::Result<crate::raw_stack::RawStack> {
        debug_assert_eq!(expected.parameters.len(), arguments.len());
        anyhow::ensure!(
            same_signature(self.0.signature.as_deref(), Some(expected)),
            "closure signature mismatch"
        );
        let mut body = self
            .0
            .body
            .try_borrow_mut()
            .map_err(|_| anyhow::anyhow!("closure body is already executing"))?;
        unsafe { body.segment()?.execute_raw(arguments) }
    }

    /// Executes this carrier with a construction-validated, representation-matched frame.
    ///
    /// This path deliberately omits the external signature check. The prepared
    /// invocation owns a clone of this exact carrier, preventing substitution.
    ///
    /// # Safety
    ///
    /// Preparation must validate the actual body's independent origin, argument reads,
    /// and result against this closure's full signature and its canonical physical
    /// frame. Every pointer must remain a live matching borrowed value
    /// until execution returns; operations must not retain argument pointers.
    /// A trusted-binding lease must protect the validated body from modification
    /// throughout preparation and execution.
    ///
    /// # Errors
    ///
    /// Returns reentrant body borrow or body evaluation errors.
    /// - Complexity: Body-dependent time and independent evaluation stack storage.
    #[cfg(test)]
    pub(crate) unsafe fn call_raw_trusted(
        &self,
        arguments: &[*const u8],
    ) -> anyhow::Result<crate::raw_stack::RawStack> {
        debug_assert_eq!(arguments.len(), self.0.param_types.len());
        debug_assert!(self.0.signature.is_some());
        let mut body = self
            .0
            .body
            .try_borrow_mut()
            .map_err(|_| anyhow::anyhow!("closure body is already executing"))?;
        unsafe { body.segment()?.execute_raw(arguments) }
    }

    /// Returns this closure's declared parameter types, in order.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::{DynClosure, DynSegment};
    /// use std::any::TypeId;
    ///
    /// let mut body = DynSegment::new::<()>();
    /// body.push_arg::<i32>(0);
    /// body.push_arg::<i32>(1);
    /// body.op2(|a: i32, b: i32| a + b).unwrap();
    ///
    /// let closure = DynClosure::new(
    ///     vec![TypeId::of::<i32>(), TypeId::of::<i32>()],
    ///     TypeId::of::<i32>(),
    ///     body,
    /// );
    /// assert_eq!(closure.param_types(), &[TypeId::of::<i32>(), TypeId::of::<i32>()]);
    /// ```
    #[must_use]
    pub fn param_types(&self) -> &[TypeId] {
        &self.0.param_types
    }

    /// Returns this closure's declared return type.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::{DynClosure, DynSegment};
    /// use std::any::TypeId;
    ///
    /// let mut body = DynSegment::new::<()>();
    /// body.push_arg::<i32>(0);
    /// body.push_arg::<i32>(1);
    /// body.op2(|a: i32, b: i32| a + b).unwrap();
    ///
    /// let closure = DynClosure::new(
    ///     vec![TypeId::of::<i32>(), TypeId::of::<i32>()],
    ///     TypeId::of::<i32>(),
    ///     body,
    /// );
    /// assert_eq!(closure.return_type(), TypeId::of::<i32>());
    /// ```
    #[must_use]
    pub fn return_type(&self) -> TypeId {
        self.0.return_type
    }

    /// Invokes the closure's body with `args`, positionally matched against `param_types`.
    ///
    /// - Precondition: `args.len() == self.param_types().len()` and each `args[i]`'s runtime type
    ///   matches `param_types()[i]` — the caller (adam-lang's generated `Filter` wrapper, or any
    ///   future caller) must guarantee this ahead of time; a violation is a caller bug, not user
    ///   error (matches `DynSegment::push_arg`'s own existing precondition, which this delegates
    ///   to unchanged).
    /// - Precondition: `TypeId::of::<R>() == self.return_type()`.
    /// - Complexity: whatever the body's own evaluation complexity is.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::{DynClosure, DynSegment};
    /// use std::any::TypeId;
    ///
    /// let mut body = DynSegment::new::<()>();
    /// body.push_arg::<i32>(0);
    /// body.push_arg::<i32>(1);
    /// body.op2(|a: i32, b: i32| a + b).unwrap();
    ///
    /// let closure = DynClosure::new(
    ///     vec![TypeId::of::<i32>(), TypeId::of::<i32>()],
    ///     TypeId::of::<i32>(),
    ///     body,
    /// );
    /// let (a, b) = (2i32, 3i32);
    /// let result: i32 = closure.call(&[&a, &b]).unwrap();
    /// assert_eq!(result, 5);
    /// ```
    pub fn call<R: 'static>(&self, args: &[&dyn Any]) -> anyhow::Result<R> {
        debug_assert_eq!(args.len(), self.0.param_types.len());
        self.0
            .body
            .try_borrow_mut()
            .map_err(|_| anyhow::anyhow!("closure body is already executing"))?
            .segment()?
            .call_dyn::<R>(args)
    }

    /// Invokes the closure's body with `args`, like [`call`](Self::call), for a caller that only
    /// knows the return type dynamically (as a `TypeId`) rather than as a static Rust generic —
    /// `call_dyn_fn` is a monomorphized dispatcher the caller already has for that type (e.g.
    /// `adam-lang`'s per-type `TypeRegistry::TypeEntry::call_dyn_fn`).
    ///
    /// - Precondition: `args.len() == self.param_types().len()` and each `args[i]`'s runtime type
    ///   matches `param_types()[i]`, exactly as for [`call`](Self::call).
    /// - Precondition: `call_dyn_fn` is a dispatcher for `self.return_type()` (i.e. it calls
    ///   `DynSegment::call_dyn::<R>` for the same concrete `R` that `TypeId` names).
    /// - Complexity: whatever the body's own evaluation complexity is.
    ///
    /// # Errors
    ///
    /// Returns an error without invoking `call_dyn_fn` while any trusted binding
    /// leases this shared body, including through another clone. Mutable dispatch
    /// resumes after the last lease drops. Otherwise propagates dispatcher errors.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cel_runtime::{DynClosure, DynSegment};
    /// use std::any::{Any, TypeId};
    ///
    /// let mut body = DynSegment::new::<()>();
    /// body.push_arg::<i32>(0);
    /// body.push_arg::<i32>(1);
    /// body.op2(|a: i32, b: i32| a + b).unwrap();
    ///
    /// let closure = DynClosure::new(
    ///     vec![TypeId::of::<i32>(), TypeId::of::<i32>()],
    ///     TypeId::of::<i32>(),
    ///     body,
    /// );
    ///
    /// fn dispatch_i32(seg: &mut DynSegment, inputs: &[&dyn Any]) -> anyhow::Result<Box<dyn Any>> {
    ///     let result: i32 = seg.call_dyn(inputs)?;
    ///     Ok(Box::new(result))
    /// }
    ///
    /// let (a, b) = (4i32, 5i32);
    /// let result = closure.call_boxed(&[&a, &b], dispatch_i32).unwrap();
    /// assert_eq!(*result.downcast_ref::<i32>().unwrap(), 9);
    /// ```
    pub fn call_boxed(
        &self,
        args: &[&dyn Any],
        call_dyn_fn: CallDynFn,
    ) -> anyhow::Result<Box<dyn Any>> {
        debug_assert_eq!(args.len(), self.0.param_types.len());
        anyhow::ensure!(
            self.0.trusted_bindings.get() == 0,
            "closure body is frozen by a trusted binding"
        );
        let mut body = self
            .0
            .body
            .try_borrow_mut()
            .map_err(|_| anyhow::anyhow!("closure body is already executing"))?;
        call_dyn_fn(body.segment()?, args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dyn_segment::DynSegment;

    /// Clones and drops the existing shared carrier without erasing its signature.
    #[test]
    fn complete_factory_callable_owns_shared_carrier() {
        use crate::value_type::ValueType as Complete;
        use std::mem::{ManuallyDrop, MaybeUninit};
        let mut body = DynSegment::new::<()>();
        body.just(42i32);
        let closure = DynClosure::typed(vec![], body).unwrap();
        let weak = Rc::downgrade(&closure.0);
        let descriptor = Complete::callable(vec![], Complete::leaf::<i32>());
        assert_eq!(descriptor.layout(), std::alloc::Layout::new::<DynClosure>());
        assert!(descriptor.can_clone());
        assert!(descriptor.can_debug());
        assert!(!descriptor.can_compare());
        let mut source = ManuallyDrop::new(closure);
        let source_ptr = (&mut *source as *mut DynClosure).cast::<u8>();
        let mut destination = MaybeUninit::<DynClosure>::uninit();
        // SAFETY: Both locations have the statically known carrier layout and
        // the descriptor describes this closure's signature.
        unsafe {
            crate::owned_value::clone_raw(&descriptor, source_ptr, destination.as_mut_ptr().cast());
            crate::owned_value::drop_raw(&descriptor, source_ptr);
            assert!(weak.upgrade().is_some());
            assert_eq!(destination.assume_init_ref().call::<i32>(&[]).unwrap(), 42);
            crate::owned_value::drop_raw(&descriptor, destination.as_mut_ptr().cast());
        }
        assert!(weak.upgrade().is_none());
    }

    /// Keeps tuple and array identities in migrated signatures.
    #[test]
    fn complete_factory_preserves_structural_callable_signatures() {
        use crate::value_type::{ValueKind, ValueType as Complete};
        let mut body = DynSegment::new::<()>();
        body.just(1i32);
        body.make_tuple(1, 0).unwrap();
        let parameter = ValueType::array(ValueType::cloneable_leaf::<i32>());
        let closure = DynClosure::typed(vec![parameter], body).unwrap();
        let descriptor = closure
            .complete_value_type(|id| {
                (id == TypeId::of::<i32>()).then(Complete::cloneable_leaf::<i32>)
            })
            .unwrap();
        let ValueKind::Callable { parameters, result } = descriptor.kind() else {
            panic!("expected complete callable metadata");
        };
        assert!(parameters[0].same_shape(&Complete::array(Complete::leaf::<i32>())));
        assert!(!parameters[0].same_shape(&Complete::array(Complete::leaf::<u32>())));
        assert!(matches!(result.kind(), ValueKind::Tuple(fields) if fields.len() == 1));
        assert!(!result.same_shape(&Complete::leaf::<i32>()));
        assert!(!descriptor.same_shape(&Complete::callable(
            vec![Complete::array(Complete::leaf::<u32>())],
            (**result).clone()
        )));
        assert!(!descriptor.same_shape(&Complete::callable(
            parameters.clone(),
            Complete::leaf::<i32>()
        )));
    }

    /// Retains known nested callable results and distinguishes canonical source unit.
    #[test]
    fn complete_factory_preserves_nested_callable_and_unit() {
        use crate::value_type::{Nil, ValueKind, ValueRepr, ValueType as Complete};
        let mut inner_body = DynSegment::new::<()>();
        inner_body.make_tuple(0, 0).unwrap();
        let inner = DynClosure::typed(vec![ValueType::leaf::<()>()], inner_body).unwrap();
        let mut body = DynSegment::new::<()>();
        body.push_closure(inner).unwrap();
        let outer = DynClosure::typed(vec![], body).unwrap();
        let complete = outer
            .complete_value_type(|id| (id == TypeId::of::<()>()).then(Complete::leaf::<()>))
            .unwrap();
        let ValueKind::Callable { result, .. } = complete.kind() else {
            panic!("expected callable");
        };
        let ValueKind::Callable { parameters, result } = result.kind() else {
            panic!("expected nested callable");
        };
        assert!(parameters[0].same_shape(&Complete::leaf::<()>()));
        assert!(!parameters[0].same_shape(&Nil::value_type()));
        assert!(result.same_shape(&Nil::value_type()));
        assert!(!result.same_shape(&Complete::leaf::<()>()));
    }

    /// Rejects unavailable signatures and missing or incorrectly keyed leaf factories.
    #[test]
    fn complete_factory_rejects_unknown_callable_metadata() {
        use crate::value_type::ValueType as Complete;
        assert!(
            adder_closure()
                .complete_value_type(|_| Some(Complete::leaf::<i32>()))
                .is_err()
        );
        let mut body = DynSegment::new::<()>();
        body.just(1i32);
        let closure = DynClosure::typed(vec![], body).unwrap();
        assert!(closure.complete_value_type(|_| None).is_err());
        assert!(
            closure
                .complete_value_type(|_| Some(Complete::leaf::<u32>()))
                .is_err()
        );
        assert!(
            closure
                .complete_value_type(|_| Some(Complete::array(Complete::leaf::<i32>())))
                .is_err()
        );

        let mut body = DynSegment::new::<()>();
        body.just(adder_closure());
        let closure = DynClosure::typed(vec![], body).unwrap();
        assert!(
            closure
                .complete_value_type(|_| Some(Complete::leaf::<DynClosure>()))
                .is_err()
        );
    }

    fn adder_closure() -> DynClosure {
        let mut body = DynSegment::new::<()>();
        body.push_arg::<i32>(0);
        body.push_arg::<i32>(1);
        body.op2(|a: i32, b: i32| a + b).unwrap();
        DynClosure::new(
            vec![TypeId::of::<i32>(), TypeId::of::<i32>()],
            TypeId::of::<i32>(),
            body,
        )
    }

    #[test]
    fn call_invokes_body_with_positional_args() {
        let closure = adder_closure();
        let (a, b) = (2i32, 3i32);
        let result: i32 = closure.call(&[&a, &b]).unwrap();
        assert_eq!(result, 5);
    }

    #[test]
    fn call_is_repeatable_with_different_args() {
        let closure = adder_closure();
        let (a1, b1) = (2i32, 3i32);
        assert_eq!(closure.call::<i32>(&[&a1, &b1]).unwrap(), 5);
        let (a2, b2) = (10i32, 20i32);
        assert_eq!(closure.call::<i32>(&[&a2, &b2]).unwrap(), 30);
    }

    #[test]
    fn clone_shares_the_same_body_and_both_remain_callable() {
        let closure = adder_closure();
        let cloned = closure.clone();
        let (a, b) = (1i32, 1i32);
        assert_eq!(closure.call::<i32>(&[&a, &b]).unwrap(), 2);
        assert_eq!(cloned.call::<i32>(&[&a, &b]).unwrap(), 2);
    }

    #[test]
    fn call_boxed_dispatches_through_a_supplied_call_dyn_fn() {
        let closure = adder_closure();
        fn call_dyn_fn(seg: &mut DynSegment, inputs: &[&dyn Any]) -> anyhow::Result<Box<dyn Any>> {
            let v: i32 = seg.call_dyn(inputs)?;
            Ok(Box::new(v))
        }
        let (a, b) = (4i32, 5i32);
        let result = closure.call_boxed(&[&a, &b], call_dyn_fn).unwrap();
        assert_eq!(*result.downcast_ref::<i32>().unwrap(), 9);
    }

    #[test]
    fn param_types_and_return_type_are_queryable() {
        let closure = adder_closure();
        assert_eq!(
            closure.param_types(),
            &[TypeId::of::<i32>(), TypeId::of::<i32>()]
        );
        assert_eq!(closure.return_type(), TypeId::of::<i32>());
    }

    /// A typed body cannot reinterpret an argument through a different Rust representation.
    #[test]
    fn typed_rejects_incompatible_body_argument_reads() {
        let mut body = DynSegment::new::<()>();
        body.push_arg::<String>(0);
        assert!(DynClosure::typed(vec![ValueType::leaf::<i32>()], body).is_err());
    }

    /// An undeclared positional argument cannot enter a typed raw call frame.
    #[test]
    fn typed_rejects_undeclared_body_argument_reads() {
        let mut body = DynSegment::new::<()>();
        body.push_arg::<i32>(1);
        assert!(DynClosure::typed(vec![ValueType::leaf::<i32>()], body).is_err());
    }

    /// A shifted fragment cannot become a body evaluated on an empty stack.
    #[test]
    fn typed_rejects_ambient_dependent_body_without_execution() {
        let mut inner = DynSegment::new::<()>();
        inner.just(42i32);
        let closure = DynClosure::typed(vec![], inner).unwrap();
        let mut parent = DynSegment::new::<()>();
        parent.just(7u64);
        parent.just(true);
        let mut fragment = parent.new_fragment();
        fragment.push_closure(closure).unwrap();
        fragment.invoke_closure(0).unwrap();

        assert!(DynClosure::typed(vec![], fragment).is_err());
    }

    /// A zero-origin fragment contains only its local operands, even above ambient unit values.
    #[test]
    fn typed_accepts_zero_origin_body_with_nested_invocation() {
        let mut inner = DynSegment::new::<()>();
        inner.just(42i32);
        let closure = DynClosure::typed(vec![], inner).unwrap();
        let mut parent = DynSegment::new::<()>();
        parent.just(());
        parent.just(true);
        let mut fragment = parent.new_fragment();
        fragment.push_closure(closure).unwrap();
        fragment.invoke_closure(0).unwrap();

        let closure = DynClosure::typed(vec![], fragment).unwrap();
        assert_eq!(closure.call::<i32>(&[]).unwrap(), 42);
        let mut caller = DynSegment::new::<()>();
        caller.push_closure(closure).unwrap();
        caller.invoke_closure(0).unwrap();
        assert_eq!(caller.call0::<i32>().unwrap(), 42);
    }

    /// A root body cannot hide a shifted fragment behind a conditional join.
    #[test]
    fn typed_body_join_rejects_mismatched_fragment_origins_without_execution() {
        for shifted_first in [true, false] {
            let mut parent = DynSegment::new::<()>();
            parent.just(7u64);
            parent.just(true);
            let mut shifted = parent.new_fragment();
            shifted.just(42i32);
            let mut body = DynSegment::new::<()>();
            body.just(true);
            let mut alternate = body.new_fragment();
            alternate.just(0i32);

            let result = if shifted_first {
                body.join2(shifted, alternate)
            } else {
                body.join2(alternate, shifted)
            };
            assert!(result.is_err());
        }
    }

    /// A runtime closure value cannot masquerade as another known callable signature.
    #[test]
    fn invocation_checks_runtime_signature_before_body_execution() {
        let mut expected_body = DynSegment::new::<()>();
        expected_body.just(1i32);
        let expected = DynClosure::typed(vec![], expected_body).unwrap();
        let executed = Rc::new(std::cell::Cell::new(false));
        let observed = executed.clone();
        let mut actual_body = DynSegment::new::<()>();
        actual_body.op0(move || {
            observed.set(true);
            true
        });
        let actual = DynClosure::typed(vec![], actual_body).unwrap();
        // Both signatures have no parameters, so the borrowed argument frame is valid.
        let error = unsafe { actual.call_raw(&expected.signature().unwrap(), &[]) }.unwrap_err();
        assert!(error.to_string().contains("signature mismatch"));
        assert!(!executed.get());
    }

    /// Argument reads inside alternate fragments participate in signature validation.
    #[test]
    fn typed_checks_conditional_argument_reads() {
        let mut body = DynSegment::new::<()>();
        body.just(true);
        let mut then_branch = body.new_fragment();
        then_branch.push_arg::<String>(0);
        let mut else_branch = body.new_fragment();
        else_branch.just(String::new());
        body.join2(then_branch, else_branch).unwrap();
        assert!(DynClosure::typed(vec![ValueType::leaf::<i32>()], body).is_err());
    }

    /// Rejected invocation leaves the evaluated operand descriptors and operations unchanged.
    #[test]
    fn wrong_argument_type_is_rejected_before_emitting() {
        let mut body = DynSegment::new::<()>();
        body.push_arg::<i32>(0);
        let closure = DynClosure::typed(vec![ValueType::leaf::<i32>()], body).unwrap();
        let mut caller = DynSegment::new::<()>();
        caller.push_closure(closure).unwrap();
        caller.just(false);
        assert!(caller.invoke_closure(1).is_err());
        assert_eq!(caller.peek_stack_infos(2).len(), 2);
        caller.op1(|_: bool| 8i32).unwrap();
        caller.invoke_closure(1).unwrap();
        assert_eq!(caller.call0::<i32>().unwrap(), 8);
    }

    /// Preloaded Rust argument layouts cannot enter the positional CEL invocation adapter.
    #[test]
    fn preloaded_arguments_fail_explicitly() {
        let mut body = DynSegment::new::<()>();
        body.just(1i32);
        let mut caller = DynSegment::new::<(i32,)>();
        caller
            .push_closure(DynClosure::typed(vec![], body).unwrap())
            .unwrap();
        assert!(caller.invoke_closure(0).is_err());
    }
}
