use crate::protocol::RelayKernelPresence;
pub(super) fn kernel_is_permitted(
    kernel: &RelayKernelPresence,
    targets: Option<&[String]>,
) -> bool {
    targets.is_none_or(|targets| targets.contains(&kernel.kernel_id))
}
