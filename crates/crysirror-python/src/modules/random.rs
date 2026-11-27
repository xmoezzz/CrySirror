// SPDX-License-Identifier: MPL-2.0
//! `_random.Random`: CPython's MT19937 generator with the same seeding,
//! `random()`, `getrandbits()` and `_randbelow()` so seeded scripts produce
//! the original sequences. Python 2 code uses the 2.7 `randrange`/`choice`/
//! `shuffle` algorithms (`int(random() * n)`).

use super::{module_with, set};
use crate::builtins::{parse, Kw};
use crate::dict::Dict;
use crate::object::*;
use crate::ops;
use crate::vm::{PyResult, Vm};
use crate::Dialect;
use num_bigint::{BigInt, Sign};
use num_traits::{Signed, ToPrimitive, Zero};
use std::cell::RefCell;

const N: usize = 624;

#[derive(Clone)]
pub struct Mt {
    pub mt: [u32; N],
    pub idx: usize,
    pub gauss_next: Option<f64>,
}

impl Mt {
    pub fn new() -> Mt {
        let mut m = Mt { mt: [0; N], idx: N + 1, gauss_next: None };
        m.init_genrand(19650218);
        m
    }
    fn init_genrand(&mut self, s: u32) {
        self.mt[0] = s;
        for i in 1..N {
            self.mt[i] = 1812433253u32.wrapping_mul(self.mt[i - 1] ^ (self.mt[i - 1] >> 30)).wrapping_add(i as u32);
        }
        self.idx = N;
    }
    pub fn init_by_array(&mut self, key: &[u32]) {
        self.init_genrand(19650218);
        let (mut i, mut j) = (1usize, 0usize);
        let klen = key.len().max(1);
        let mut k = N.max(klen);
        while k > 0 {
            self.mt[i] = (self.mt[i] ^ (self.mt[i - 1] ^ (self.mt[i - 1] >> 30)).wrapping_mul(1664525)).wrapping_add(*key.get(j).unwrap_or(&0)).wrapping_add(j as u32);
            i += 1;
            j += 1;
            if i >= N {
                self.mt[0] = self.mt[N - 1];
                i = 1;
            }
            if j >= klen {
                j = 0;
            }
            k -= 1;
        }
        k = N - 1;
        while k > 0 {
            self.mt[i] = (self.mt[i] ^ (self.mt[i - 1] ^ (self.mt[i - 1] >> 30)).wrapping_mul(1566083941)).wrapping_sub(i as u32);
            i += 1;
            if i >= N {
                self.mt[0] = self.mt[N - 1];
                i = 1;
            }
            k -= 1;
        }
        self.mt[0] = 0x8000_0000;
        self.idx = N;
    }
    pub fn genrand(&mut self) -> u32 {
        if self.idx >= N {
            for kk in 0..N {
                let y = (self.mt[kk] & 0x8000_0000) | (self.mt[(kk + 1) % N] & 0x7fff_ffff);
                let mag = if y & 1 != 0 { 0x9908_b0df } else { 0 };
                self.mt[kk] = self.mt[(kk + 397) % N] ^ (y >> 1) ^ mag;
            }
            self.idx = 0;
        }
        let mut y = self.mt[self.idx];
        self.idx += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c_5680;
        y ^= (y << 15) & 0xefc6_0000;
        y ^ (y >> 18)
    }
    pub fn random(&mut self) -> f64 {
        let a = (self.genrand() >> 5) as f64;
        let b = (self.genrand() >> 6) as f64;
        (a * 67108864.0 + b) * (1.0 / 9007199254740992.0)
    }
    pub fn getrandbits(&mut self, k: u64) -> BigInt {
        if k <= 32 {
            return BigInt::from(self.genrand() >> (32 - k));
        }
        let words = k.div_ceil(32) as usize;
        let mut ws = vec![0u32; words];
        let mut left = k;
        for w in ws.iter_mut() {
            let mut r = self.genrand();
            if left < 32 {
                r >>= 32 - left;
            }
            *w = r;
            left = left.saturating_sub(32);
        }
        BigInt::from_slice(Sign::Plus, &ws)
    }
    pub fn randbelow(&mut self, n: &BigInt) -> BigInt {
        if n.is_zero() {
            return BigInt::zero();
        }
        let k = n.bits();
        loop {
            let r = self.getrandbits(k);
            if &r < n {
                return r;
            }
        }
    }
    pub fn seed_int(&mut self, n: &BigInt) {
        let (_, words) = n.abs().to_u32_digits();
        let words = if words.is_empty() { vec![0] } else { words };
        self.init_by_array(&words);
        self.gauss_next = None;
    }
}

impl Default for Mt {
    fn default() -> Self {
        Mt::new()
    }
}

fn mt_of(vm: &Vm, v: &Value) -> PyResult<ObjRef> {
    let Value::Obj(o) = v else { return Err(vm.type_error("Random instance expected")) };
    match ops::get_raw_attr(o, "__mt__") {
        Some(Value::Obj(m)) => Ok(m),
        _ => Err(vm.type_error("Random instance not initialised")),
    }
}

fn with_mt<R>(vm: &Vm, v: &Value, f: impl FnOnce(&mut Mt) -> R) -> PyResult<R> {
    let m = mt_of(vm, v)?;
    let Payload::Native(n) = &m.payload else { return Err(vm.type_error("bad state")) };
    let mut b = n.borrow_mut();
    let mt = b.downcast_mut::<Mt>().ok_or_else(|| vm.type_error("bad state"))?;
    Ok(f(mt))
}

pub fn random_f(vm: &Vm, v: &Value) -> PyResult<f64> {
    with_mt(vm, v, |m| m.random())
}

fn r_init(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let Value::Obj(o) = &a[0] else { unreachable!() };
    if ops::get_raw_attr(o, "__mt__").is_none() {
        let st = Value::Obj(new_obj(vm.types.native.clone(), None, Payload::Native(RefCell::new(Box::new(Mt::new())))));
        ops::set_raw_attr(vm, o, "__mt__", st);
    }
    let seed = vm.types.intern("seed");
    let f = ops::getattr_v(vm, &a[0], &seed)?;
    vm.call(&f, vec![a.get(1).cloned().unwrap_or(Value::None)], Vec::new())?;
    Ok(Value::None)
}

fn r_seed(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let [x, version] = parse(vm, "seed", &a[1..], k, ["a", "version"], 0)?;
    let x = x.unwrap_or(Value::None);
    let version = crate::builtins::opt_i64(vm, &version, 2)?;
    let n: BigInt = if x.is_none() {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        BigInt::from(t)
    } else if let Some(b) = ops::to_bigint(&x) {
        b
    } else if let Some(s) = x.as_str() {
        if version == 2 && ops::vm_dialect(vm) == Dialect::Py3 {
            // CPython 3: int.from_bytes(a + sha512(a).digest(), 'big').
            let mut bytes = s.as_bytes().to_vec();
            bytes.extend(crate::modules::hashlib::sha512(s.as_bytes()));
            BigInt::from_bytes_be(Sign::Plus, &bytes)
        } else {
            BigInt::from(ops::hash(vm, &x)?)
        }
    } else if let Some(b) = ops::bytes_of(&x) {
        let mut bytes = b.clone();
        bytes.extend(crate::modules::hashlib::sha512(&b));
        BigInt::from_bytes_be(Sign::Plus, &bytes)
    } else {
        BigInt::from(ops::hash(vm, &x)?)
    };
    with_mt(vm, &a[0], |m| m.seed_int(&n))?;
    Ok(Value::None)
}

fn r_random(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    // Subclasses may override random(); callers below use `call_random`.
    Ok(Value::Float(random_f(vm, &a[0])?))
}

/// self.random(), honouring Python-level overrides.
fn call_random(vm: &mut Vm, slf: &Value) -> PyResult<f64> {
    let t = ops::type_of(vm, slf);
    if let Some(f) = ops::lookup_user(&t, "random") {
        let r = vm.call(&f, vec![slf.clone()], Vec::new())?;
        return ops::to_f64(vm, &r);
    }
    random_f(vm, slf)
}

fn r_getrandbits(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let k = ops::index_of(vm, &a[1])?;
    if k < 0 {
        return Err(vm.value_error("number of bits must be non-negative"));
    }
    let b = with_mt(vm, &a[0], |m| m.getrandbits(k as u64))?;
    Ok(vm.types.new_bigint(b))
}

fn randbelow(vm: &mut Vm, slf: &Value, n: &BigInt) -> PyResult<BigInt> {
    if ops::vm_dialect(vm) == Dialect::Py2 {
        let r = call_random(vm, slf)?;
        return Ok(num_traits::FromPrimitive::from_f64((r * n.to_f64().unwrap_or(0.0)).floor()).unwrap_or_default());
    }
    let t = ops::type_of(vm, slf);
    if ops::lookup_user(&t, "random").is_some() && ops::lookup_user(&t, "getrandbits").is_none() {
        let r = call_random(vm, slf)?;
        return Ok(num_traits::FromPrimitive::from_f64((r * n.to_f64().unwrap_or(0.0)).floor()).unwrap_or_default());
    }
    with_mt(vm, slf, |m| m.randbelow(n))
}

fn r_randrange(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let [start, stop, step] = parse(vm, "randrange", &a[1..], k, ["start", "stop", "step"], 1)?;
    let start = ops::to_bigint(start.as_ref().unwrap()).ok_or_else(|| vm.value_error("non-integer arg 1 for randrange()"))?;
    let stop = match stop.filter(|v| !v.is_none()) {
        None => {
            if !start.is_positive() {
                return Err(vm.value_error("empty range for randrange()"));
            }
            let r = randbelow(vm, &a[0], &start)?;
            return Ok(vm.types.new_bigint(r));
        }
        Some(s) => ops::to_bigint(&s).ok_or_else(|| vm.value_error("non-integer stop for randrange()"))?,
    };
    let step = match step {
        Some(s) => ops::to_bigint(&s).ok_or_else(|| vm.value_error("non-integer step for randrange()"))?,
        None => BigInt::from(1),
    };
    let width = &stop - &start;
    if step == BigInt::from(1) {
        if !width.is_positive() {
            return Err(vm.value_error(format!("empty range for randrange() ({start}, {stop}, {width})")));
        }
        let r = randbelow(vm, &a[0], &width)?;
        return Ok(vm.types.new_bigint(start + r));
    }
    if step.is_zero() {
        return Err(vm.value_error("zero step for randrange()"));
    }
    let n: BigInt = if step.is_positive() { (&width + &step - 1) / &step } else { (&width + &step + 1) / &step };
    if !n.is_positive() {
        return Err(vm.value_error("empty range for randrange()"));
    }
    let r = randbelow(vm, &a[0], &n)?;
    Ok(vm.types.new_bigint(start + step * r))
}

fn r_randint(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    crate::builtins::arg_count(vm, "randint", a, 3, 3)?;
    let b = ops::binary_op(vm, &a[2], &Value::Int(1), crate::ast::BinOp::Add, false, false)?;
    r_randrange(vm, &[a[0].clone(), a[1].clone(), b], &[])
}

fn r_choice(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let n = ops::len(vm, &a[1])?;
    if n == 0 {
        return Err(vm.index_error("Cannot choose from an empty sequence"));
    }
    let i = randbelow(vm, &a[0], &BigInt::from(n))?;
    ops::getitem(vm, &a[1], Value::Int(i.to_i64().unwrap_or(0)))
}

fn r_shuffle(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let n = ops::len(vm, &a[1])?;
    for i in (1..n).rev() {
        let j = randbelow(vm, &a[0], &BigInt::from(i + 1))?.to_i64().unwrap_or(0);
        let x = ops::getitem(vm, &a[1], Value::Int(i as i64))?;
        let y = ops::getitem(vm, &a[1], Value::Int(j))?;
        ops::setitem(vm, &a[1], Value::Int(i as i64), y)?;
        ops::setitem(vm, &a[1], Value::Int(j), x)?;
    }
    Ok(Value::None)
}

fn r_sample(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let [pop, kk, counts] = parse(vm, "sample", &a[1..], k, ["population", "k", "counts"], 2)?;
    let mut pool = ops::collect(vm, pop.as_ref().unwrap())?;
    if let Some(c) = counts.filter(|v| !v.is_none()) {
        let cs = ops::collect(vm, &c)?;
        let mut expanded = Vec::new();
        for (p, c) in pool.iter().zip(cs) {
            for _ in 0..ops::index_of(vm, &c)? {
                expanded.push(p.clone());
            }
        }
        pool = expanded;
    }
    let n = pool.len();
    let kk = ops::index_of(vm, kk.as_ref().unwrap())?;
    if kk < 0 || kk as usize > n {
        return Err(vm.value_error("Sample larger than population or is negative"));
    }
    let kk = kk as usize;
    let mut out = Vec::with_capacity(kk);
    // CPython's algorithm choice between a pool and a selected-set.
    let mut setsize = 21usize;
    if kk > 5 {
        setsize += 4usize.pow(((kk * 3) as f64).log(4.0).ceil() as u32);
    }
    if n <= setsize {
        for i in 0..kk {
            let j = randbelow(vm, &a[0], &BigInt::from(n - i))?.to_usize().unwrap();
            out.push(pool[j].clone());
            pool[j] = pool[n - i - 1].clone();
        }
    } else {
        let mut selected = std::collections::HashSet::new();
        for _ in 0..kk {
            let mut j = randbelow(vm, &a[0], &BigInt::from(n))?.to_usize().unwrap();
            while selected.contains(&j) {
                j = randbelow(vm, &a[0], &BigInt::from(n))?.to_usize().unwrap();
            }
            selected.insert(j);
            out.push(pool[j].clone());
        }
    }
    Ok(vm.types.new_list(out))
}

fn r_choices(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let [pop, weights, cum, kk] = parse(vm, "choices", &a[1..], k, ["population", "weights", "cum_weights", "k"], 1)?;
    let pool = ops::collect(vm, pop.as_ref().unwrap())?;
    let n = pool.len();
    let kk = crate::builtins::opt_i64(vm, &kk, 1)?.max(0) as usize;
    let mut out = Vec::with_capacity(kk);
    let cum: Option<Vec<f64>> = match (weights.filter(|v| !v.is_none()), cum.filter(|v| !v.is_none())) {
        (None, None) => None,
        (Some(w), None) => {
            let mut acc = 0.0;
            let mut c = Vec::new();
            for x in ops::collect(vm, &w)? {
                acc += ops::to_f64(vm, &x)?;
                c.push(acc);
            }
            Some(c)
        }
        (None, Some(c)) => {
            let mut v = Vec::new();
            for x in ops::collect(vm, &c)? {
                v.push(ops::to_f64(vm, &x)?);
            }
            Some(v)
        }
        _ => return Err(vm.type_error("Cannot specify both weights and cumulative weights")),
    };
    if n == 0 {
        return Err(vm.index_error("Cannot choose from an empty population"));
    }
    match cum {
        None => {
            for _ in 0..kk {
                let r = call_random(vm, &a[0])?;
                out.push(pool[((r * n as f64).floor() as usize).min(n - 1)].clone());
            }
        }
        Some(c) => {
            let total = *c.last().unwrap_or(&0.0);
            for _ in 0..kk {
                let r = call_random(vm, &a[0])? * total;
                let i = c.partition_point(|x| *x <= r).min(n - 1);
                out.push(pool[i].clone());
            }
        }
    }
    Ok(vm.types.new_list(out))
}

fn r_uniform(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let x = ops::to_f64(vm, &a[1])?;
    let y = ops::to_f64(vm, &a[2])?;
    let r = call_random(vm, &a[0])?;
    Ok(Value::Float(x + (y - x) * r))
}

fn r_triangular(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let [low, high, mode] = parse(vm, "triangular", &a[1..], k, ["low", "high", "mode"], 0)?;
    let mut low = match low {
        Some(v) => ops::to_f64(vm, &v)?,
        None => 0.0,
    };
    let mut high = match high {
        Some(v) => ops::to_f64(vm, &v)?,
        None => 1.0,
    };
    let mut u = call_random(vm, &a[0])?;
    let c = match mode.filter(|v| !v.is_none()) {
        Some(m) => {
            if high == low {
                return Ok(Value::Float(low));
            }
            (ops::to_f64(vm, &m)? - low) / (high - low)
        }
        None => 0.5,
    };
    let mut c = c;
    if u > c {
        u = 1.0 - u;
        c = 1.0 - c;
        std::mem::swap(&mut low, &mut high);
    }
    Ok(Value::Float(low + (high - low) * (u * c).sqrt()))
}

fn r_gauss(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let mu = match a.get(1) {
        Some(v) => ops::to_f64(vm, v)?,
        None => 0.0,
    };
    let sigma = match a.get(2) {
        Some(v) => ops::to_f64(vm, v)?,
        None => 1.0,
    };
    let saved = with_mt(vm, &a[0], |m| m.gauss_next.take())?;
    let z = match saved {
        Some(z) => z,
        None => {
            let x2pi = call_random(vm, &a[0])? * std::f64::consts::TAU;
            let g2rad = (-2.0 * (1.0 - call_random(vm, &a[0])?).ln()).sqrt();
            let z = x2pi.cos() * g2rad;
            with_mt(vm, &a[0], |m| m.gauss_next = Some(x2pi.sin() * g2rad))?;
            z
        }
    };
    Ok(Value::Float(mu + z * sigma))
}

fn r_normalvariate(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let mu = match a.get(1) {
        Some(v) => ops::to_f64(vm, v)?,
        None => 0.0,
    };
    let sigma = match a.get(2) {
        Some(v) => ops::to_f64(vm, v)?,
        None => 1.0,
    };
    let nv_magicconst = 4.0 * (-0.5f64).exp() / 2.0f64.sqrt();
    loop {
        let u1 = call_random(vm, &a[0])?;
        let u2 = 1.0 - call_random(vm, &a[0])?;
        let z = nv_magicconst * (u1 - 0.5) / u2;
        let zz = z * z / 4.0;
        if zz <= -u2.ln() {
            return Ok(Value::Float(mu + z * sigma));
        }
    }
}

fn r_lognormvariate(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let r = r_normalvariate(vm, a, k)?;
    Ok(Value::Float(ops::to_f64(vm, &r)?.exp()))
}

fn r_expovariate(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let lambd = match a.get(1) {
        Some(v) => ops::to_f64(vm, v)?,
        None => 1.0,
    };
    let r = call_random(vm, &a[0])?;
    Ok(Value::Float(-(1.0 - r).ln() / lambd))
}

fn r_paretovariate(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let alpha = ops::to_f64(vm, &a[1])?;
    let u = 1.0 - call_random(vm, &a[0])?;
    Ok(Value::Float(u.powf(-1.0 / alpha)))
}

fn r_weibullvariate(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let alpha = ops::to_f64(vm, &a[1])?;
    let beta = ops::to_f64(vm, &a[2])?;
    let u = 1.0 - call_random(vm, &a[0])?;
    Ok(Value::Float(alpha * (-u.ln()).powf(1.0 / beta)))
}

fn gamma_sample(vm: &mut Vm, slf: &Value, alpha: f64, beta: f64) -> PyResult<f64> {
    if alpha <= 0.0 || beta <= 0.0 {
        return Err(vm.value_error("gammavariate: alpha and beta must be > 0.0"));
    }
    if alpha > 1.0 {
        let ainv = (2.0 * alpha - 1.0).sqrt();
        let bbb = alpha - 4f64.ln();
        let ccc = alpha + ainv;
        let sg_magicconst = 1.0 + 4.5f64.ln();
        loop {
            let u1 = call_random(vm, slf)?;
            if !(1e-7 < u1 && u1 < 0.9999999) {
                continue;
            }
            let u2 = 1.0 - call_random(vm, slf)?;
            let v = (u1 / (1.0 - u1)).ln() / ainv;
            let x = alpha * v.exp();
            let z = u1 * u1 * u2;
            let r = bbb + ccc * v - x;
            if r + sg_magicconst - 4.5 * z >= 0.0 || r >= z.ln() {
                return Ok(x * beta);
            }
        }
    } else if alpha == 1.0 {
        Ok(-(1.0 - call_random(vm, slf)?).ln() * beta)
    } else {
        loop {
            let u = call_random(vm, slf)?;
            let b = (std::f64::consts::E + alpha) / std::f64::consts::E;
            let p = b * u;
            let x = if p <= 1.0 { p.powf(1.0 / alpha) } else { -((b - p) / alpha).ln() };
            let u1 = call_random(vm, slf)?;
            if p > 1.0 {
                if u1 <= x.powf(alpha - 1.0) {
                    return Ok(x * beta);
                }
            } else if u1 <= (-x).exp() {
                return Ok(x * beta);
            }
        }
    }
}

fn r_gammavariate(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let alpha = ops::to_f64(vm, &a[1])?;
    let beta = ops::to_f64(vm, &a[2])?;
    Ok(Value::Float(gamma_sample(vm, &a[0], alpha, beta)?))
}

fn r_betavariate(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let alpha = ops::to_f64(vm, &a[1])?;
    let beta = ops::to_f64(vm, &a[2])?;
    let y = gamma_sample(vm, &a[0], alpha, 1.0)?;
    if y == 0.0 {
        return Ok(Value::Float(0.0));
    }
    let z = gamma_sample(vm, &a[0], beta, 1.0)?;
    Ok(Value::Float(y / (y + z)))
}

fn r_vonmisesvariate(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let mu = ops::to_f64(vm, &a[1])?;
    let kappa = ops::to_f64(vm, &a[2])?;
    if kappa <= 1e-6 {
        return Ok(Value::Float(std::f64::consts::TAU * call_random(vm, &a[0])?));
    }
    let s = 0.5 / kappa;
    let r = s + (1.0 + s * s).sqrt();
    let z = loop {
        let u1 = call_random(vm, &a[0])?;
        let z = (std::f64::consts::PI * u1).cos();
        let d = z / (r + z);
        let u2 = call_random(vm, &a[0])?;
        if u2 < 1.0 - d * d || u2 <= (1.0 - d) * d.exp() {
            break z;
        }
    };
    let q = 1.0 / r;
    let f = (q + z) / (1.0 + q * z);
    let u3 = call_random(vm, &a[0])?;
    let theta = if u3 > 0.5 { (mu + f.acos()).rem_euclid(std::f64::consts::TAU) } else { (mu - f.acos()).rem_euclid(std::f64::consts::TAU) };
    Ok(Value::Float(theta))
}

fn r_getstate(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let (words, idx, g) = with_mt(vm, &a[0], |m| (m.mt.to_vec(), m.idx, m.gauss_next))?;
    let mut items: Vec<Value> = words.into_iter().map(|w| Value::Int(w as i64)).collect();
    items.push(Value::Int(idx as i64));
    let inner = vm.types.new_tuple(items);
    let gn = g.map(Value::Float).unwrap_or(Value::None);
    Ok(vm.types.new_tuple(vec![Value::Int(3), inner, gn]))
}

fn r_setstate(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let st = ops::collect(vm, &a[1])?;
    if st.len() != 3 {
        return Err(vm.value_error("state with version 3 expected"));
    }
    let inner = ops::collect(vm, &st[1])?;
    if inner.len() != N + 1 {
        return Err(vm.value_error("state vector is the wrong size"));
    }
    let mut mt = [0u32; N];
    for (i, w) in inner[..N].iter().enumerate() {
        mt[i] = ops::index_of(vm, w)? as u32;
    }
    let idx = ops::index_of(vm, &inner[N])? as usize;
    let g = if st[2].is_none() { None } else { Some(ops::to_f64(vm, &st[2])?) };
    with_mt(vm, &a[0], |m| {
        m.mt = mt;
        m.idx = idx.min(N);
        m.gauss_next = g;
    })?;
    Ok(Value::None)
}

fn r_randbytes(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let n = ops::index_of(vm, &a[1])?.max(0) as u64;
    let b = with_mt(vm, &a[0], |m| m.getrandbits(n * 8))?;
    let mut bytes = b.to_bytes_le().1;
    bytes.resize(n as usize, 0);
    Ok(vm.types.new_bytes(bytes))
}

fn r_reduce(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let cls = Value::Obj(ops::type_of(vm, &a[0]));
    let st = r_getstate(vm, &a[..1], &[])?;
    let empty = vm.types.new_tuple(Vec::new());
    Ok(vm.types.new_tuple(vec![cls, empty, st]))
}

fn r_setstate_pickle(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let Value::Obj(o) = &a[0] else { unreachable!() };
    if ops::get_raw_attr(o, "__mt__").is_none() {
        let st = Value::Obj(new_obj(vm.types.native.clone(), None, Payload::Native(RefCell::new(Box::new(Mt::new())))));
        ops::set_raw_attr(vm, o, "__mt__", st);
    }
    r_setstate(vm, a, k)
}

pub fn init(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = module_with(vm, "_random", &[]);
    let c = vm.types.new_builtin_type("Random", &vm.types.object.clone(), BaseKind::Object, false);
    for (n, f) in [
        ("__init__", r_init as NativeFn), ("seed", r_seed), ("random", r_random), ("getrandbits", r_getrandbits),
        ("randrange", r_randrange), ("randint", r_randint), ("choice", r_choice), ("shuffle", r_shuffle), ("sample", r_sample),
        ("choices", r_choices), ("uniform", r_uniform), ("triangular", r_triangular), ("gauss", r_gauss),
        ("normalvariate", r_normalvariate), ("lognormvariate", r_lognormvariate), ("expovariate", r_expovariate),
        ("paretovariate", r_paretovariate), ("weibullvariate", r_weibullvariate), ("gammavariate", r_gammavariate),
        ("betavariate", r_betavariate), ("vonmisesvariate", r_vonmisesvariate), ("getstate", r_getstate),
        ("setstate", r_setstate), ("randbytes", r_randbytes), ("__reduce__", r_reduce), ("__setstate__", r_setstate_pickle),
    ] {
        vm.types.add_method(&c, n, f);
    }
    let mn = vm.types.new_str("random");
    vm.types.set_attr(&c, "__module__", mn);
    vm.classes.random = Some(c.clone());
    set(vm, &m, "Random", Value::Obj(c));
    let _ = Dict::new;
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cpython_sequence() {
        // random.seed(42); random.random() == 0.6394267984578837
        let mut m = Mt::new();
        m.seed_int(&BigInt::from(42));
        assert_eq!(m.random(), 0.6394267984578837);
        assert_eq!(m.random(), 0.025010755222666936);
    }
}
