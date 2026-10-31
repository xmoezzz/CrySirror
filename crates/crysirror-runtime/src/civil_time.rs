// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! OS/timezone reads happen on a bounded shared worker, never a VM/owner tick.
use chrono::{Datelike, Timelike};
use crysirror_core::{
    service::{CivilTime, CivilTimeQuery, CivilTimeState},
    state::ClientId,
    Error, Result,
};
use crysirror_tasks::{Task, TaskPool};
struct Query {
    client: ClientId,
    task: Option<Task<CivilTime>>,
    state: CivilTimeState,
}
impl CivilTimeQuery for Query {
    fn client(&self) -> ClientId {
        self.client
    }
    fn poll(&mut self) -> Result<CivilTimeState> {
        if let Some(task) = &mut self.task {
            match task.try_take() {
                Ok(None) => {}
                Ok(Some(value)) => {
                    self.state = CivilTimeState::Ready(value);
                    self.task = None;
                }
                Err(e) => {
                    self.state = CivilTimeState::Failed(e.to_string());
                    self.task = None;
                }
            }
        }
        Ok(self.state.clone())
    }
}
pub(crate) fn begin(pool: &TaskPool, client: ClientId) -> Result<Box<dyn CivilTimeQuery>> {
    let task = pool
        .spawn(move |cancel| {
            cancel.check()?;
            let t = chrono::Local::now();
            let result = CivilTime {
                client,
                unix_ms: t.timestamp_millis(),
                utc_offset_seconds: t.offset().local_minus_utc(),
                year: t.year(),
                month: t.month() as u8,
                day: t.day() as u8,
                hour: t.hour() as u8,
                minute: t.minute() as u8,
                second: t.second() as u8,
                millisecond: t.timestamp_subsec_millis() as u16,
            };
            cancel.check()?;
            Ok(result)
        })
        .map_err(|e| Error::Io(format!("civil clock admission: {e:#}")))?;
    Ok(Box::new(Query {
        client,
        task: Some(task),
        state: CivilTimeState::Pending,
    }))
}
