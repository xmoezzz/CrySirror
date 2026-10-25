// SPDX-License-Identifier: MPL-2.0
use crate::{
    EngineContext, Value,
    object::{Class, ClassInfo, ClassRegistry, MethodInfo, ObjectId, TypeHint as T},
};
use anyhow::Result;
use std::sync::{Arc, OnceLock};
pub fn register(r: &mut ClassRegistry) -> Result<()> {
    r.register(Arc::new(ProfilerClass))
}
struct ProfilerClass;
impl Class for ProfilerClass {
    fn info(&self) -> &ClassInfo {
        static INFO: OnceLock<ClassInfo> = OnceLock::new();
        INFO.get_or_init(||ClassInfo{name:"Profiler",doc:"Bounded engine CPU timings and optional completed GPU timestamp samples. Diagnostics never serialize device state into project saves.",properties:vec![],methods:vec![
            MethodInfo{name:"snapshot",params:vec![],returns:T::Map,doc:"Stage sample counts, mean/p50/p95/max microseconds and dropped-sample counts. GPU and CPU clocks are not calibrated."},
            MethodInfo{name:"trace",params:vec![],returns:T::Map,doc:"Export retained CPU spans as Chrome trace JSON, including worker threads and frame IDs. GPU durations are reported separately by snapshot."},
            MethodInfo{name:"clear",params:vec![],returns:T::Void,doc:"Clear retained diagnostics without resetting the engine frame identity or simulation."},
        ]})
    }
    fn get(&self, _: &mut EngineContext, _: ObjectId, _: &str) -> Result<Value> {
        unreachable!()
    }
    fn call(&self, cx: &mut EngineContext, _: ObjectId, name: &str, _: &[Value]) -> Result<Value> {
        let p = cx.state()?.runtime.profiler();
        Ok(match name {
            "snapshot" => Value::from_json(p.snapshot()),
            "trace" => Value::from_json(p.chrome_trace()),
            "clear" => {
                p.clear();
                Value::Null
            }
            _ => unreachable!(),
        })
    }
}
