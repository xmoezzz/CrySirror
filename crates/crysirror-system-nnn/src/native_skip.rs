// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Original script-flow skip flags. These are native control/save policy;
//! input mapping and reusable text/image scheduling belong to engine services.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NativeSkipPolicy {
    pub f4: bool,
    pub f5: bool,
    pub next_command: bool,
    pub effect_command: bool,
    pub film_end: bool,
}
impl NativeSkipPolicy {
    /// Original ReceiveUserFunction0 stopf5 resets all five flags, independent
    /// of supplied parameter count (reviewed legacy function 0x5355c0).
    pub fn stop_f5(&mut self) { *self = Self::default(); }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stop_f5_resets_every_native_skip_mode() {
        let mut policy = NativeSkipPolicy { f4:true,f5:true,next_command:true,effect_command:true,film_end:true };
        policy.stop_f5();
        assert_eq!(policy, NativeSkipPolicy::default());
    }
}
