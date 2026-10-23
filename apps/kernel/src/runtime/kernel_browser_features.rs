//! MP-08/MP-11: the Room browser mirror and display are on for every kernel.
//! The environment variables are kill switches only: "0" turns a feature off.

const MIRROR: &str = "CHARIOX_KERNEL_BROWSER_MIRROR";
const DISPLAY: &str = "CHARIOX_KERNEL_BROWSER_DISPLAY";

fn enabled(name: &str) -> bool {
    std::env::var(name).as_deref() != Ok("0")
}

pub(crate) fn mirror_enabled() -> bool {
    enabled(MIRROR)
}

pub(crate) fn display_enabled() -> bool {
    enabled(DISPLAY)
}

/// The resolved values the browser host reads, so its gates agree with the kernel's.
pub(crate) fn host_environment() -> [(&'static str, &'static str); 2] {
    let flag = |on| if on { "1" } else { "0" };
    [
        (MIRROR, flag(mirror_enabled())),
        (DISPLAY, flag(display_enabled())),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn features_are_on_unless_the_kill_switch_is_zero() {
        crate::test_support::isolated_env_test!();
        std::env::remove_var(MIRROR);
        std::env::remove_var(DISPLAY);
        assert!(mirror_enabled() && display_enabled());
        assert_eq!(
            host_environment(),
            [(MIRROR, "1"), (DISPLAY, "1")],
            "the host must receive the resolved defaults"
        );
        std::env::set_var(MIRROR, "0");
        std::env::set_var(DISPLAY, "0");
        assert!(!mirror_enabled() && !display_enabled());
        assert_eq!(host_environment(), [(MIRROR, "0"), (DISPLAY, "0")]);
        std::env::set_var(MIRROR, "1");
        std::env::remove_var(DISPLAY);
        assert_eq!(host_environment(), [(MIRROR, "1"), (DISPLAY, "1")]);
    }
}
