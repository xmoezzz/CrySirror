// SPDX-License-Identifier: MPL-2.0
//! `time`. Wall time comes from the host clock; local time is UTC plus the
//! offset the embedding engine configures (`set_utc_offset`).

use super::{module_with, set};
use crate::builtins::Kw;
use crate::dict::Dict;
use crate::object::*;
use crate::ops;
use crate::vm::{PyResult, Vm};
use std::time::{SystemTime, UNIX_EPOCH};

pub struct TimeState {
    pub utc_offset: i64,
    pub start: std::time::Instant,
}

fn state(vm: &Vm) -> (i64, std::time::Instant) {
    match vm.ext_get::<TimeState>() {
        Some(s) => (s.utc_offset, s.start),
        None => (0, std::time::Instant::now()),
    }
}

pub fn set_utc_offset(vm: &Vm, secs: i64) {
    let start = state(vm).1;
    vm.ext_set(TimeState { utc_offset: secs, start });
}

pub fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

fn time(_vm: &mut Vm, _a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(Value::Float(now()))
}

fn time_ns(vm: &mut Vm, _a: &[Value], _k: Kw) -> PyResult<Value> {
    let ns = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    Ok(vm.types.new_bigint(num_bigint::BigInt::from(ns)))
}

fn perf_counter(vm: &mut Vm, _a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(Value::Float(state(vm).1.elapsed().as_secs_f64()))
}

fn sleep(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let s = ops::to_f64(vm, a.first().unwrap_or(&Value::Int(0)))?;
    if s < 0.0 {
        return Err(vm.value_error("sleep length must be non-negative"));
    }
    // Script threads never block the engine; sleeping is a no-op.
    Ok(Value::None)
}

/// (year, month, day) from days since 1970-01-01.
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 } as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// Broken-down time: (year, mon, mday, hour, min, sec, wday[Mon=0], yday[1..]).
pub fn breakdown(t: i64) -> [i64; 8] {
    let days = t.div_euclid(86400);
    let secs = t.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    let wday = (days + 3).rem_euclid(7);
    let yday = days - days_from_civil(y, 1, 1) + 1;
    [y, m as i64, d as i64, secs / 3600, (secs / 60) % 60, secs % 60, wday, yday]
}

fn struct_time(vm: &mut Vm, b: [i64; 8], isdst: i64) -> Value {
    let cls = struct_time_type(vm);
    let mut items: Vec<Value> = b.iter().map(|x| Value::Int(*x)).collect();
    items.push(Value::Int(isdst));
    let names = ["tm_year", "tm_mon", "tm_mday", "tm_hour", "tm_min", "tm_sec", "tm_wday", "tm_yday", "tm_isdst"];
    let mut d = Dict::new();
    for (n, v) in names.iter().zip(&items) {
        d.set_str(vm.types.intern(n), v.clone());
    }
    Value::Obj(new_obj(cls, Some(d), Payload::Tuple(items.into_boxed_slice())))
}

fn struct_time_type(vm: &mut Vm) -> ObjRef {
    if let Some(Value::Obj(m)) = crate::import::get_module(vm, "time") {
        if let Some(Value::Obj(t)) = m.dict_get_str_raw("struct_time") {
            return t;
        }
    }
    vm.types.new_builtin_type("struct_time", &vm.types.tuple.clone(), BaseKind::Tuple, false)
}

fn arg_time(vm: &mut Vm, a: &[Value]) -> PyResult<i64> {
    match a.first() {
        Some(v) if !v.is_none() => Ok(ops::to_f64(vm, v)?.floor() as i64),
        _ => Ok(now().floor() as i64),
    }
}

fn localtime(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let t = arg_time(vm, a)? + state(vm).0;
    Ok(struct_time(vm, breakdown(t), 0))
}

fn gmtime(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let t = arg_time(vm, a)?;
    Ok(struct_time(vm, breakdown(t), 0))
}

fn tm_parts(vm: &mut Vm, v: &Value) -> PyResult<[i64; 9]> {
    let items = ops::collect(vm, v)?;
    if items.len() < 9 {
        return Err(vm.type_error("time tuple must have 9 elements"));
    }
    let mut out = [0i64; 9];
    for (i, it) in items.iter().take(9).enumerate() {
        out[i] = ops::index_of(vm, it)?;
    }
    Ok(out)
}

fn mktime(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let p = tm_parts(vm, &a[0])?;
    let days = days_from_civil(p[0], p[1].clamp(1, 12) as u32, 1) + p[2] - 1;
    let t = days * 86400 + p[3] * 3600 + p[4] * 60 + p[5] - state(vm).0;
    Ok(Value::Float(t as f64))
}

const DAYS: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

pub fn strftime_impl(fmt: &str, p: &[i64; 9]) -> String {
    let mut out = String::new();
    let mut it = fmt.chars().peekable();
    while let Some(c) = it.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let Some(d) = it.next() else {
            out.push('%');
            break;
        };
        let wd = p[6].rem_euclid(7) as usize;
        let mo = (p[1].clamp(1, 12) - 1) as usize;
        match d {
            'Y' => out.push_str(&p[0].to_string()),
            'y' => out.push_str(&format!("{:02}", p[0].rem_euclid(100))),
            'm' => out.push_str(&format!("{:02}", p[1])),
            'd' => out.push_str(&format!("{:02}", p[2])),
            'e' => out.push_str(&format!("{:2}", p[2])),
            'H' => out.push_str(&format!("{:02}", p[3])),
            'I' => out.push_str(&format!("{:02}", if p[3] % 12 == 0 { 12 } else { p[3] % 12 })),
            'M' => out.push_str(&format!("{:02}", p[4])),
            'S' => out.push_str(&format!("{:02}", p[5])),
            'p' => out.push_str(if p[3] < 12 { "AM" } else { "PM" }),
            'A' => out.push_str(DAYS[wd]),
            'a' => out.push_str(&DAYS[wd][..3]),
            'B' => out.push_str(MONTHS[mo]),
            'b' | 'h' => out.push_str(&MONTHS[mo][..3]),
            'j' => out.push_str(&format!("{:03}", p[7])),
            'w' => out.push_str(&((wd + 1) % 7).to_string()),
            'u' => out.push_str(&(wd + 1).to_string()),
            'c' => out.push_str(&format!("{} {} {:2} {:02}:{:02}:{:02} {}", &DAYS[wd][..3], &MONTHS[mo][..3], p[2], p[3], p[4], p[5], p[0])),
            'x' => out.push_str(&format!("{:02}/{:02}/{:02}", p[1], p[2], p[0].rem_euclid(100))),
            'X' => out.push_str(&format!("{:02}:{:02}:{:02}", p[3], p[4], p[5])),
            'Z' => out.push_str("UTC"),
            'z' => out.push_str("+0000"),
            'U' | 'W' => {
                let yday = p[7] - 1;
                let first = if d == 'U' { ((wd as i64 + 1) % 7 - yday).rem_euclid(7) } else { (wd as i64 - yday).rem_euclid(7) };
                let _ = is_leap(p[0]);
                out.push_str(&format!("{:02}", (yday + first) / 7));
            }
            '%' => out.push('%'),
            other => {
                out.push('%');
                out.push(other);
            }
        }
    }
    out
}

fn strftime(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let fmt = crate::builtins::str_arg(vm, &a[0], "format")?;
    let p = match a.get(1) {
        Some(t) => tm_parts(vm, t)?,
        None => {
            let b = breakdown(now().floor() as i64 + state(vm).0);
            [b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], 0]
        }
    };
    let s = strftime_impl(&fmt, &p);
    Ok(vm.types.new_string(s))
}

fn asctime(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let p = match a.first() {
        Some(t) => tm_parts(vm, t)?,
        None => {
            let b = breakdown(now().floor() as i64 + state(vm).0);
            [b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], 0]
        }
    };
    let s = strftime_impl("%a %b %e %H:%M:%S %Y", &p);
    Ok(vm.types.new_string(s))
}

fn ctime(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let t = arg_time(vm, a)? + state(vm).0;
    let b = breakdown(t);
    let s = strftime_impl("%a %b %e %H:%M:%S %Y", &[b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], 0]);
    Ok(vm.types.new_string(s))
}

pub fn init(vm: &mut Vm) -> PyResult<ObjRef> {
    if vm.ext_get::<TimeState>().is_none() {
        vm.ext_set(TimeState { utc_offset: 0, start: std::time::Instant::now() });
    }
    let m = module_with(vm, "time", &[
        ("time", time),
        ("time_ns", time_ns),
        ("perf_counter", perf_counter),
        ("monotonic", perf_counter),
        ("process_time", perf_counter),
        ("clock", perf_counter),
        ("sleep", sleep),
        ("localtime", localtime),
        ("gmtime", gmtime),
        ("mktime", mktime),
        ("strftime", strftime),
        ("asctime", asctime),
        ("ctime", ctime),
    ]);
    let st = vm.types.new_builtin_type("struct_time", &vm.types.tuple.clone(), BaseKind::Tuple, false);
    set(vm, &m, "struct_time", Value::Obj(st));
    set(vm, &m, "timezone", Value::Int(-state(vm).0));
    set(vm, &m, "altzone", Value::Int(-state(vm).0));
    set(vm, &m, "daylight", Value::Int(0));
    let tz = vm.types.new_tuple(vec![vm.types.new_str("UTC"), vm.types.new_str("UTC")]);
    set(vm, &m, "tzname", tz);
    Ok(m)
}
