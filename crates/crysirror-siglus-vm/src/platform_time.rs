// SPDX-License-Identifier: MPL-2.0
//! Per-call injected engine clock; scopes restore the previous client clock.
pub use std::time::Duration;
use std::{
    cell::Cell,
    ops::{Add, Sub},
};
thread_local! {static NOW:Cell<u64>=const{Cell::new(0)};}
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
)]
pub struct Instant(pub u64);
impl Instant {
    pub fn now() -> Self {
        NOW.with(|n| Self(n.get()))
    }
    pub fn elapsed(&self) -> Duration {
        Self::now().saturating_duration_since(*self)
    }
    pub fn duration_since(&self, old: Self) -> Duration {
        self.saturating_duration_since(old)
    }
    pub fn saturating_duration_since(&self, old: Self) -> Duration {
        Duration::from_millis(self.0.saturating_sub(old.0))
    }
    pub fn checked_add(&self, d: Duration) -> Option<Self> {
        self.0
            .checked_add(u64::try_from(d.as_millis()).ok()?)
            .map(Self)
    }
}
impl Add<Duration> for Instant {
    type Output = Self;
    fn add(self, d: Duration) -> Self {
        Self(
            self.0
                .saturating_add(d.as_millis().min(u64::MAX as u128) as u64),
        )
    }
}
impl Sub<Instant> for Instant {
    type Output = Duration;
    fn sub(self, x: Self) -> Duration {
        self.duration_since(x)
    }
}
pub fn scoped<T>(ms: u64, f: impl FnOnce() -> T) -> T {
    struct Reset(u64);
    impl Drop for Reset {
        fn drop(&mut self) {
            NOW.with(|n| n.set(self.0));
        }
    }
    let _reset = Reset(NOW.with(|n| n.replace(ms)));
    f()
}
#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct LocalTimeFields {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub weekday_sunday0: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub millisecond: u32,
}
pub fn local_time_fields() -> LocalTimeFields {
    use chrono::{Datelike, Timelike};
    let now = chrono::Local::now();
    LocalTimeFields {
        year: now.year(),
        month: now.month(),
        day: now.day(),
        weekday_sunday0: now.weekday().num_days_from_sunday(),
        hour: now.hour(),
        minute: now.minute(),
        second: now.second(),
        millisecond: now.timestamp_subsec_millis(),
    }
}
