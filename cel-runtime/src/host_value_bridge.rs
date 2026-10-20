//! Checks the single erased whole-value host boundary.
//!
//! Native scalar boxes retain their concrete types. Compound values cross this
//! boundary as `OwnedValue`, preserving canonical storage without conversion.

use std::any::Any;
use std::borrow::Cow;

use crate::owned_value::{Allocation, OwnedValue};
use crate::{ValueKind, ValueType};

/// Retains concrete scalar box moves without optional capability requirements.
#[derive(Clone, Debug)]
pub(crate) struct LeafBridge {
    pub(crate) name: Cow<'static, str>,
    write: unsafe fn(Box<dyn Any>, *mut u8),
}

impl LeafBridge {
    /// Borrows the registered scalar name.
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    /// Binds a concrete scalar box move without imposing trait bounds.
    pub(crate) fn new<T: 'static>() -> Self {
        Self {
            name: Cow::Borrowed(std::any::type_name::<T>()),
            write: |value, destination| unsafe {
                destination
                    .cast::<T>()
                    .write(*value.downcast::<T>().unwrap());
            },
        }
    }
}

/// Validates scalar identity or complete canonical ownership before execution.
pub(crate) struct HostValuePlan {
    value_type: ValueType,
}

impl HostValuePlan {
    /// Binds a complete canonical descriptor.
    ///
    /// # Errors
    /// Rejects an erased owner registered as an opaque element.
    /// - Complexity: O(n) in descriptor nodes.
    pub(crate) fn new(value_type: ValueType) -> anyhow::Result<Self> {
        value_type.validate_canonical()?;
        Ok(Self { value_type })
    }

    /// Returns borrowed canonical storage after validating the complete input.
    ///
    /// # Errors
    /// Rejects scalar identity, compound shape, or clone-capability mismatches.
    /// - Complexity: O(n) in descriptor nodes.
    pub(crate) fn validate_any(&self, input: &dyn Any) -> anyhow::Result<*const u8> {
        anyhow::ensure!(
            self.value_type.can_clone_for_argument(),
            "host argument clone capability unavailable"
        );
        if matches!(self.value_type.kind(), ValueKind::Leaf(_)) {
            self.value_type.validate_opaque_leaf(input.type_id())?;
            Ok(input as *const dyn Any as *const u8)
        } else {
            let owner = input
                .downcast_ref::<OwnedValue>()
                .ok_or_else(|| anyhow::anyhow!("canonical whole-value owner required"))?;
            anyhow::ensure!(
                self.value_type.same_shape(owner.value_type()),
                "canonical host argument shape mismatch"
            );
            Ok(owner.as_ptr())
        }
    }

    /// Imports a scalar box or canonical owner, retaining the input on mismatch.
    ///
    /// # Errors
    /// Rejects missing input, wrong scalar identity, or wrong compound shape.
    /// - Complexity: O(n) in descriptor nodes plus one scalar raw move.
    pub(crate) fn import_box(
        &self,
        input: &mut Option<Box<dyn Any>>,
    ) -> anyhow::Result<OwnedValue> {
        let value = input
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("missing external owner"))?;
        if matches!(self.value_type.kind(), ValueKind::Leaf(_)) {
            self.value_type.validate_opaque_leaf(value.type_id())?;
            let allocation = Allocation::new(self.value_type.layout());
            // SAFETY: Exact concrete identity precedes the monomorphized move.
            unsafe {
                (self.value_type.host_leaf().unwrap().write)(
                    input.take().unwrap(),
                    allocation.pointer.as_ptr(),
                );
                Ok(OwnedValue::from_allocation(
                    allocation,
                    self.value_type.clone(),
                ))
            }
        } else {
            let owner = value
                .downcast_ref::<OwnedValue>()
                .ok_or_else(|| anyhow::anyhow!("canonical whole-value owner required"))?;
            anyhow::ensure!(
                self.value_type.same_shape(owner.value_type()),
                "canonical host argument shape mismatch"
            );
            (*input.take().unwrap().downcast::<OwnedValue>().unwrap())
                .with_value_type(self.value_type.clone())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Array, ArrayOf, Cons, IntoCStackList, Leaf, Nil, ValueRepr};

    /// A rejected complete shape retains the original owner and its allocation.
    #[test]
    fn nested_shape_mismatch_preserves_the_entire_input() {
        type Actual = Cons<ArrayOf<Leaf<String>>, Nil>;
        type Expected = Cons<ArrayOf<Leaf<i32>>, Nil>;
        let strings = Array::from_vec(vec![String::from("owned")]);
        let pointer = strings.as_slice().as_ptr();
        let value = OwnedValue::new::<Actual>((strings,).into_c_stack_list());
        let mut input: Option<Box<dyn Any>> = Some(Box::new(value));
        let wrong = HostValuePlan::new(Expected::value_type()).unwrap();
        assert!(wrong.import_box(&mut input).is_err());
        let right = HostValuePlan::new(Actual::value_type()).unwrap();
        let value = right.import_box(&mut input).unwrap();
        assert!(input.is_none());
        use crate::IntoRustTuple;
        let (array,) = value.try_into_typed::<Actual>().unwrap().into_rust_tuple();
        assert_eq!(array.as_slice().as_ptr(), pointer);
        assert_eq!(array.as_slice(), ["owned"]);
    }

    /// Empty arrays retain complete element identity even without payload values.
    #[test]
    fn empty_array_shape_and_missing_inputs_are_checked() {
        let owner = OwnedValue::new::<ArrayOf<Leaf<i32>>>(Array::from_vec(vec![]));
        let wrong =
            HostValuePlan::new(ValueType::array(ValueType::cloneable_leaf::<u32>())).unwrap();
        assert!(wrong.validate_any(&owner).is_err());
        let plan =
            HostValuePlan::new(ValueType::array(ValueType::cloneable_leaf::<i32>())).unwrap();
        assert!(plan.validate_any(&owner).is_ok());
        assert!(plan.import_box(&mut None).is_err());
        let mut scalar: Option<Box<dyn Any>> = Some(Box::new(1i32));
        assert!(plan.import_box(&mut scalar).is_err());
        assert_eq!(scalar.unwrap().downcast_ref::<i32>(), Some(&1));
    }

    /// Import moves foreign drop-only leaves without adding capability requirements.
    #[test]
    fn move_only_leaf_import_keeps_its_payload_allocation() {
        let value = Box::new(std::sync::Mutex::new(String::from("move only")));
        let pointer = &*value as *const _;
        let plan = HostValuePlan::new(ValueType::leaf::<Box<std::sync::Mutex<String>>>()).unwrap();
        let mut input: Option<Box<dyn Any>> = Some(Box::new(value));
        let owner = plan.import_box(&mut input).unwrap();
        assert!(input.is_none());
        assert!(owner.try_clone().is_err());
        let value = owner
            .try_into_typed::<Leaf<Box<std::sync::Mutex<String>>>>()
            .unwrap();
        assert_eq!(&*value as *const _, pointer);
        assert_eq!(*value.lock().unwrap(), "move only");
    }

    /// Equal-sized foreign identities cannot substitute for a concrete scalar.
    #[test]
    fn scalar_identity_mismatch_retains_the_input() {
        let plan = HostValuePlan::new(ValueType::leaf::<std::num::Wrapping<u64>>()).unwrap();
        let mut input: Option<Box<dyn Any>> = Some(Box::new(7u64));
        assert!(plan.import_box(&mut input).is_err());
        assert_eq!(input.unwrap().downcast_ref::<u64>(), Some(&7));
    }
}
