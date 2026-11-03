//! Native display-policy callbacks. This is data, never an OS/GPU operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeDisplayAction {
    RegisterMode {
        index: usize,
        width: i32,
        height: i32,
    },
    WindowedSize {
        width: i32,
        height: i32,
    },
    ScaleMode {
        mode: i32,
    },
    MonitorMode {
        mode: i32,
    },
}
