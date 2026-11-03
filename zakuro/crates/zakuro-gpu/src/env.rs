//! switches read from the environment once.

use std::sync::OnceLock;

/// an environment variable that turns something on when it is set to
/// anything but 0, read the first time it is asked for.
pub(crate) struct EnvFlag {
    name: &'static str,
    on: OnceLock<bool>,
}

impl EnvFlag {
    pub(crate) const fn new(name: &'static str) -> EnvFlag {
        EnvFlag { name, on: OnceLock::new() }
    }

    /// whether the variable is set to anything but 0.
    pub(crate) fn on(&self) -> bool {
        *self.on.get_or_init(|| set_and_not_zero(std::env::var(self.name).ok().as_deref()))
    }
}

/// whether a variable's value turns its switch on: set, and not 0.
fn set_and_not_zero(value: Option<&str>) -> bool {
    value.is_some_and(|value| value != "0")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_switch_is_on_when_set_to_anything_but_zero() {
        assert!(!set_and_not_zero(None));
        assert!(!set_and_not_zero(Some("0")));
        assert!(set_and_not_zero(Some("1")));
        assert!(set_and_not_zero(Some("")));
        assert!(!EnvFlag::new("ZAKURO_A_SWITCH_NO_ONE_SETS").on());
    }
}
