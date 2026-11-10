//! Wasmtime 50 executes post-return before call returns. Copy bounded strings
//! during lifting, while guest allocations are still live. ABI layout, type
//! checking, bounds/alignment, UTF-8 and fuel accounting remain Wasmtime's.
use chariox_app_runtime::workflow_gate::{Failure, OUTPUT_BYTES};
use std::cell::Cell;
use wasmtime::component::{
    __internal::{CanonicalAbiInfo, InstanceType, InterfaceType, LiftContext},
    ComponentType, Lift,
};
thread_local! { static REMAINING: Cell<usize> = const { Cell::new(0) }; }
pub(super) fn begin() {
    REMAINING.with(|budget| budget.set(OUTPUT_BYTES));
}
fn charge(bytes: usize) -> wasmtime::Result<()> {
    REMAINING.with(|budget| {
        let remaining = budget.get().checked_sub(bytes).ok_or(Failure::GateLimit)?;
        budget.set(remaining);
        Ok(())
    })
}
pub(super) struct Text<const N: usize>(pub String);
pub(super) struct List<T, const N: usize>(pub Vec<T>);
// SAFETY: these wrappers preserve exactly the delegated ABI and type check.
// The additional checks only reject lengths; all actual lifting, validation
// and allocation is performed by Wasmtime's corresponding String/Vec<T> impl.
unsafe impl<const N: usize> ComponentType for Text<N> {
    type Lower = <String as ComponentType>::Lower;
    const ABI: CanonicalAbiInfo = <String as ComponentType>::ABI;
    fn typecheck(ty: &InterfaceType, types: &InstanceType<'_>) -> wasmtime::Result<()> {
        String::typecheck(ty, types)
    }
}
unsafe impl<const N: usize> Lift for Text<N> {
    fn linear_lift_from_flat(
        cx: &mut LiftContext<'_>,
        ty: InterfaceType,
        src: &Self::Lower,
    ) -> wasmtime::Result<Self> {
        let len = src[1].get_u32() as usize;
        wasmtime::ensure!(len <= N, Failure::GateLimit);
        charge(len)?;
        Ok(Self(String::linear_lift_from_flat(cx, ty, src)?))
    }
    fn linear_lift_from_memory(
        cx: &mut LiftContext<'_>,
        ty: InterfaceType,
        bytes: &[u8],
    ) -> wasmtime::Result<Self> {
        let len = u32::from_le_bytes(bytes[4..8].try_into()?) as usize;
        wasmtime::ensure!(len <= N, Failure::GateLimit);
        charge(len)?;
        Ok(Self(String::linear_lift_from_memory(cx, ty, bytes)?))
    }
}
unsafe impl<T: ComponentType, const N: usize> ComponentType for List<T, N> {
    type Lower = <Vec<T> as ComponentType>::Lower;
    const ABI: CanonicalAbiInfo = <Vec<T> as ComponentType>::ABI;
    fn typecheck(ty: &InterfaceType, types: &InstanceType<'_>) -> wasmtime::Result<()> {
        Vec::<T>::typecheck(ty, types)
    }
}
unsafe impl<T: Lift, const N: usize> Lift for List<T, N> {
    fn linear_lift_from_flat(
        cx: &mut LiftContext<'_>,
        ty: InterfaceType,
        src: &Self::Lower,
    ) -> wasmtime::Result<Self> {
        wasmtime::ensure!(src[1].get_u32() as usize <= N, Failure::GateLimit);
        Ok(Self(Vec::<T>::linear_lift_from_flat(cx, ty, src)?))
    }
    fn linear_lift_from_memory(
        cx: &mut LiftContext<'_>,
        ty: InterfaceType,
        bytes: &[u8],
    ) -> wasmtime::Result<Self> {
        wasmtime::ensure!(
            u32::from_le_bytes(bytes[4..8].try_into()?) as usize <= N,
            Failure::GateLimit
        );
        Ok(Self(Vec::<T>::linear_lift_from_memory(cx, ty, bytes)?))
    }
}
