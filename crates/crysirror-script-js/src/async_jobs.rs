// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Bounded Boa microtasks/futures and service Promise bindings. No thread waits.
use boa_engine::{
    Context, JsNativeError, JsResult, JsValue,
    job::{FutureJob, JobQueue, NativeJob},
};
use crysirror_engine::{EngineContext, async_api::AsyncCalls};
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    sync::Arc,
    task::{Context as PollContext, Poll, Wake, Waker},
};
const CAPACITY: usize = 1024;
const TURN_BUDGET: usize = 64;
#[derive(Default)]
pub(crate) struct Jobs {
    jobs: RefCell<VecDeque<NativeJob>>,
    futures: RefCell<VecDeque<FutureJob>>,
    outstanding: Cell<usize>,
    error: RefCell<Option<String>>,
}
impl Jobs {
    fn admit(&self) -> bool {
        if self.outstanding.get() >= CAPACITY {
            *self.error.borrow_mut() =
                Some("script microtask/future queue capacity exceeded".into());
            false
        } else {
            self.outstanding.set(self.outstanding.get() + 1);
            true
        }
    }
    pub fn take_error(&self) -> Option<String> {
        self.error.borrow_mut().take()
    }
    pub fn clear(&self) {
        self.jobs.borrow_mut().clear();
        self.futures.borrow_mut().clear();
        self.outstanding.set(0);
    }
}
struct NoWake;
impl Wake for NoWake {
    fn wake(self: Arc<Self>) {}
}
impl JobQueue for Jobs {
    fn enqueue_promise_job(&self, job: NativeJob, _: &mut Context) {
        if self.admit() {
            self.jobs.borrow_mut().push_back(job);
        }
    }
    fn enqueue_future_job(&self, future: FutureJob, _: &mut Context) {
        if self.admit() {
            self.futures.borrow_mut().push_back(future);
        }
    }
    fn run_jobs(&self, context: &mut Context) {
        let waker = Waker::from(Arc::new(NoWake));
        let mut cx = PollContext::from_waker(&waker);
        let count = self.futures.borrow().len().min(TURN_BUDGET);
        for _ in 0..count {
            let mut future = self.futures.borrow_mut().pop_front().unwrap();
            match future.as_mut().poll(&mut cx) {
                Poll::Ready(job) => self.jobs.borrow_mut().push_back(job),
                Poll::Pending => self.futures.borrow_mut().push_back(future),
            }
        }
        for _ in 0..TURN_BUDGET {
            let Some(job) = self.jobs.borrow_mut().pop_front() else {
                break;
            };
            self.outstanding.set(self.outstanding.get() - 1);
            if let Err(error) = job.call(context) {
                *self.error.borrow_mut() = Some(format!("script microtask failed: {error}"));
                break;
            }
        }
    }
}
thread_local! {pub(crate) static CALLS:RefCell<AsyncCalls>=RefCell::new(AsyncCalls::default());}
fn operation(
    work: impl FnOnce(&mut AsyncCalls, &mut EngineContext) -> anyhow::Result<serde_json::Value>,
) -> JsResult<JsValue> {
    let result = super::ENGINE.with(|engine| {
        CALLS.with(|calls| {
            let mut engine = engine
                .try_borrow_mut()
                .map_err(|_| anyhow::anyhow!("reentrant asynchronous engine call"))?;
            let mut calls = calls
                .try_borrow_mut()
                .map_err(|_| anyhow::anyhow!("reentrant script await"))?;
            work(&mut calls, &mut engine)
        })
    });
    Ok(JsValue::from(boa_engine::js_string!(
        serde_json::to_string(&result.map_err(super::js_err)?)
            .map_err(|e| super::js_err(e.into()))?
    )))
}
pub(crate) fn start(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let object = super::arg_id(args, context)?;
    let binding = args
        .get(1)
        .cloned()
        .unwrap_or_default()
        .to_number(context)?;
    if !binding.is_finite() || binding < 0. || binding.fract() != 0. {
        return Err(JsNativeError::typ()
            .with_message("invalid asynchronous binding")
            .into());
    }
    let raw = super::arg_string(args, 2, context)?;
    operation(|calls, cx| {
        let crysirror_engine::Value::List(args) = super::parse(&raw)? else {
            anyhow::bail!("async arguments must be an array")
        };
        calls.begin(cx, object, binding as usize, &args)
    })
}
pub(crate) fn poll(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let watch = super::arg_id(args, context)?.0;
    operation(|calls, cx| calls.poll(cx, watch))
}
pub(crate) fn cancel(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let watch = super::arg_id(args, context)?.0;
    operation(|calls, cx| {
        calls.cancel(cx, watch)?;
        Ok(serde_json::Value::Null)
    })
}
pub(crate) const PRELUDE: &str = r#"
__cs.pendingAsync = new Map();
__cs.awaitService = function(object, binding, args) {
  let info=null, resolve, reject;
  const promise=new Promise((yes,no)=>{resolve=yes;reject=no;});
  try {
    info=JSON.parse(__cs_async_start(object,binding,JSON.stringify(args)));
    __cs.pendingAsync.set(info.watch,{resolve,reject});
  } catch(error) { reject(error); }
  Object.defineProperties(promise,{
    ticket:{get(){return info===null?null:info.ticket;}},
    cancel:{value(){
      if(info===null || !__cs.pendingAsync.has(info.watch))return;
      const wait=__cs.pendingAsync.get(info.watch);__cs.pendingAsync.delete(info.watch);
      try{__cs_async_cancel(info.watch);}finally{wait.reject(new Error('engine operation cancelled'));}
    }}
  });
  return promise;
};
__cs.pumpAsync = function() {
  for(const [watch,wait] of Array.from(__cs.pendingAsync.entries())) {
    try {
      const result=JSON.parse(__cs_async_poll(watch));
      if(result.state==='ready') {__cs.pendingAsync.delete(watch);wait.resolve(result.value);}
    } catch(error) {__cs.pendingAsync.delete(watch);wait.reject(error);}
  }
};
__cs.cancelAllAsync = function() {
  for(const [watch,wait] of Array.from(__cs.pendingAsync.entries())) {
    __cs.pendingAsync.delete(watch);
    try{__cs_async_cancel(watch);}catch(error){}
    wait.reject(new Error('script host shutting down'));
  }
};
"#;
