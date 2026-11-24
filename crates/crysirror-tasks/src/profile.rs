// SPDX-License-Identifier: MPL-2.0
//! Bounded CPU spans and independently clocked GPU duration samples.
use serde::Serialize;
use std::{
    cell::RefCell,
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use web_time::Instant;

const MAX_SAMPLES: usize = 8192;
thread_local! {
    static ACTIVE: RefCell<Option<(Profiler, u64)>> = const { RefCell::new(None) };
}

struct ProfileScope(Option<(Profiler, u64)>);
impl Drop for ProfileScope {
    fn drop(&mut self) {
        ACTIVE.with(|active| {
            active.replace(self.0.take());
        });
    }
}

/// Span within the current owner-thread profile scope, if one is active.
pub fn span_current(name: &'static str) -> Option<Span> {
    ACTIVE.with(|active| {
        active
            .borrow()
            .as_ref()
            .map(|(profiler, frame)| profiler.span(*frame, name))
    })
}
#[derive(Clone, Debug, Serialize)]
pub struct Sample {
    pub frame: u64,
    pub name: String,
    pub thread: String,
    pub start_us: f64,
    pub duration_us: f64,
    pub gpu: bool,
}
struct Inner {
    origin: Instant,
    frame: AtomicU64,
    samples: Mutex<(VecDeque<Sample>, u64)>,
}
/// Shared instrumentation only; no World/device ownership or execution locks.
#[derive(Clone)]
pub struct Profiler(Arc<Inner>);
impl Default for Profiler {
    fn default() -> Self {
        Self(Arc::new(Inner {
            origin: Instant::now(),
            frame: AtomicU64::new(0),
            samples: Mutex::new((VecDeque::new(), 0)),
        }))
    }
}
impl Profiler {
    /// Attach native service spans to this frame on the current thread.
    pub fn scope<T>(&self, frame: u64, run: impl FnOnce() -> T) -> T {
        let _scope = ProfileScope(ACTIVE.with(|active| active.replace(Some((self.clone(), frame)))));
        run()
    }
    pub fn begin_frame(&self) -> u64 {
        self.0.frame.fetch_add(1, Ordering::Relaxed) + 1
    }
    pub fn frame(&self) -> u64 {
        self.0.frame.load(Ordering::Relaxed)
    }
    pub fn span(&self, frame: u64, name: impl Into<String>) -> Span {
        Span {
            profiler: self.clone(),
            frame,
            name: name.into(),
            start: Instant::now(),
        }
    }
    fn record(&self, sample: Sample) {
        let mut s = self.0.samples.lock().unwrap();
        if s.0.len() == MAX_SAMPLES {
            s.0.pop_front();
            s.1 += 1;
        }
        s.0.push_back(sample);
    }
    /// GPU durations have no calibrated CPU start timestamp. They are excluded
    /// from the CPU Chrome timeline rather than falsely aligned to submission.
    pub fn gpu_duration(&self, frame: u64, name: &str, nanoseconds: f64) {
        if nanoseconds.is_finite() && nanoseconds >= 0. {
            self.record(Sample {
                frame,
                name: name.into(),
                thread: "gpu".into(),
                start_us: 0.,
                duration_us: nanoseconds / 1000.,
                gpu: true,
            });
        }
    }
    pub fn clear(&self) {
        let mut s = self.0.samples.lock().unwrap();
        s.0.clear();
        s.1 = 0;
    }
    pub fn snapshot(&self) -> serde_json::Value {
        let s = self.0.samples.lock().unwrap();
        let mut groups = std::collections::BTreeMap::<String, Vec<f64>>::new();
        for v in &s.0 {
            groups
                .entry(format!("{}:{}", if v.gpu { "gpu" } else { "cpu" }, v.name))
                .or_default()
                .push(v.duration_us);
        }
        let stages:Vec<_>=groups.into_iter().map(|(name,mut v)| {
            v.sort_by(f64::total_cmp);let n=v.len();
            serde_json::json!({"name":name,"samples":n,"mean_us":v.iter().sum::<f64>()/n as f64,"p50_us":v[(n-1)/2],"p95_us":v[((n*95).div_ceil(100)-1).min(n-1)],"max_us":v[n-1]})
        }).collect();
        serde_json::json!({"frame":self.frame(),"retained_samples":s.0.len(),"dropped_samples":s.1,"stages":stages,"gpu_cpu_clocks_calibrated":false})
    }
    pub fn chrome_trace(&self) -> serde_json::Value {
        let s = self.0.samples.lock().unwrap();
        let events:Vec<_>=s.0.iter().filter(|v|!v.gpu).map(|v|serde_json::json!({"name":v.name,"cat":"engine","ph":"X","pid":1,"tid":v.thread,"ts":v.start_us,"dur":v.duration_us,"args":{"frame":v.frame}})).collect();
        serde_json::json!({"traceEvents":events,"displayTimeUnit":"ms","dropped_samples":s.1})
    }
}
pub struct Span {
    profiler: Profiler,
    frame: u64,
    name: String,
    start: Instant,
}
impl Drop for Span {
    fn drop(&mut self) {
        self.profiler.record(Sample {
            frame: self.frame,
            name: std::mem::take(&mut self.name),
            thread: {
                #[cfg(not(target_arch = "wasm32"))]
                { format!("{:?}", std::thread::current().id()) }
                #[cfg(target_arch = "wasm32")]
                {
                    #[cfg(feature = "web-threads")]
                    { format!("browser-cpu-{:?}", rayon::current_thread_index()) }
                    #[cfg(not(feature = "web-threads"))]
                    { "browser-worker".into() }
                }
            },
            start_us: self
                .start
                .duration_since(self.profiler.0.origin)
                .as_secs_f64()
                * 1e6,
            duration_us: self.start.elapsed().as_secs_f64() * 1e6,
            gpu: false,
        });
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_scope_restores_the_prior_frame() {
        let p = Profiler::default();
        assert!(span_current("outside").is_none());
        p.scope(1, || {
            drop(span_current("first"));
            p.scope(2, || drop(span_current("nested")));
            drop(span_current("last"));
        });
        assert!(span_current("outside").is_none());
        let trace = p.chrome_trace();
        let frames: Vec<_> = trace["traceEvents"].as_array().unwrap().iter()
            .map(|event| event["args"]["frame"].as_u64().unwrap()).collect();
        assert_eq!(frames, [1, 2, 1]);
    }
    #[test]
    fn bounded_samples_and_separate_gpu_clock_are_visible() {
        let p = Profiler::default();
        let f = p.begin_frame();
        for _ in 0..MAX_SAMPLES + 3 {
            drop(p.span(f, "work"));
        }
        p.gpu_duration(f, "frame", 2000.);
        let s = p.snapshot();
        assert_eq!(s["retained_samples"], MAX_SAMPLES);
        assert_eq!(s["dropped_samples"], 4);
        assert_eq!(
            p.chrome_trace()["traceEvents"].as_array().unwrap().len(),
            MAX_SAMPLES - 1
        );
        assert!(
            s["stages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["name"] == "gpu:frame" && s["mean_us"] == 2.)
        );
        p.clear();
        assert_eq!(p.snapshot()["retained_samples"], 0);
    }
}
