// SPDX-License-Identifier: MPL-2.0
//! `posix` (exposed as `os`) and `errno`. Path operations are pure string
//! functions; filesystem queries go through the host's scoped hooks.

use super::{module_with, set};
use crate::builtins::Kw;
use crate::object::*;
use crate::ops;
use crate::vm::{PyResult, Vm};

fn path_arg(vm: &Vm, v: &Value) -> PyResult<String> {
    if let Some(s) = v.as_str() {
        return Ok(s.to_string());
    }
    if let Some(b) = ops::bytes_of(v) {
        return Ok(String::from_utf8_lossy(&b).to_string());
    }
    Err(vm.type_error(format!("expected str, bytes or os.PathLike object, not {}", ops::type_name(vm, v))))
}

pub fn normpath(p: &str) -> String {
    if p.is_empty() {
        return ".".into();
    }
    let abs = p.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for c in p.split('/') {
        match c {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|l| *l != "..") {
                    parts.pop();
                } else if !abs {
                    parts.push("..");
                }
            }
            c => parts.push(c),
        }
    }
    let j = parts.join("/");
    if abs {
        format!("/{j}")
    } else if j.is_empty() {
        ".".into()
    } else {
        j
    }
}

fn s(vm: &Vm, v: String) -> Value {
    vm.types.new_string(v)
}

fn p_join(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let mut out = String::new();
    for x in a {
        let p = path_arg(vm, x)?;
        if p.starts_with('/') || out.is_empty() {
            out = p;
        } else if out.ends_with('/') {
            out.push_str(&p);
        } else {
            out.push('/');
            out.push_str(&p);
        }
    }
    Ok(s(vm, out))
}

fn split_p(p: &str) -> (String, String) {
    match p.rfind('/') {
        Some(i) => {
            let head = &p[..=i];
            let head = if head.chars().all(|c| c == '/') { head.to_string() } else { head.trim_end_matches('/').to_string() };
            (head, p[i + 1..].to_string())
        }
        None => (String::new(), p.to_string()),
    }
}

fn p_split(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let p = path_arg(vm, &a[0])?;
    let (h, t) = split_p(&p);
    let parts = vec![s(vm, h), s(vm, t)];
    Ok(vm.types.new_tuple(parts))
}

fn p_basename(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let p = path_arg(vm, &a[0])?;
    Ok(s(vm, split_p(&p).1))
}

fn p_dirname(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let p = path_arg(vm, &a[0])?;
    Ok(s(vm, split_p(&p).0))
}

fn splitext_p(p: &str) -> (String, String) {
    let base_start = p.rfind('/').map(|i| i + 1).unwrap_or(0);
    let base = &p[base_start..];
    let lead = base.len() - base.trim_start_matches('.').len();
    match base[lead..].rfind('.') {
        Some(i) => {
            let at = base_start + lead + i;
            (p[..at].to_string(), p[at..].to_string())
        }
        None => (p.to_string(), String::new()),
    }
}

fn p_splitext(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let p = path_arg(vm, &a[0])?;
    let (r, e) = splitext_p(&p);
    let parts = vec![s(vm, r), s(vm, e)];
    Ok(vm.types.new_tuple(parts))
}

fn p_normpath(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let p = path_arg(vm, &a[0])?;
    Ok(s(vm, normpath(&p)))
}

fn p_abspath(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let p = path_arg(vm, &a[0])?;
    let p = if p.starts_with('/') { p } else { format!("/{p}") };
    Ok(s(vm, normpath(&p)))
}

fn p_isabs(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let p = path_arg(vm, &a[0])?;
    Ok(Value::Bool(p.starts_with('/')))
}

fn p_expanduser(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(a[0].clone()).map(|v| {
        let _ = &vm;
        v
    })
}

fn p_exists(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let p = path_arg(vm, &a[0])?;
    Ok(Value::Bool(vm.host.stat(&p).is_some()))
}

fn p_isfile(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let p = path_arg(vm, &a[0])?;
    Ok(Value::Bool(vm.host.stat(&p) == Some(false)))
}

fn p_isdir(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let p = path_arg(vm, &a[0])?;
    Ok(Value::Bool(vm.host.stat(&p) == Some(true)))
}

fn p_getsize(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let p = path_arg(vm, &a[0])?;
    match vm.host.read_file(&p) {
        Some(d) => Ok(Value::Int(d.len() as i64)),
        None => Err(vm.exc_msg("FileNotFoundError", format!("[Errno 2] No such file or directory: '{p}'"))),
    }
}

fn p_getmtime(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let p = path_arg(vm, &a[0])?;
    if vm.host.stat(&p).is_none() {
        return Err(vm.exc_msg("FileNotFoundError", format!("[Errno 2] No such file or directory: '{p}'")));
    }
    Ok(Value::Float(0.0))
}

fn p_relpath(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let p = normpath(&path_arg(vm, &a[0])?);
    let start = match a.get(1) {
        Some(v) if !v.is_none() => normpath(&path_arg(vm, v)?),
        _ => ".".into(),
    };
    let pp: Vec<&str> = p.split('/').filter(|x| !x.is_empty() && *x != ".").collect();
    let sp: Vec<&str> = start.split('/').filter(|x| !x.is_empty() && *x != ".").collect();
    let common = pp.iter().zip(&sp).take_while(|(a, b)| a == b).count();
    let mut out: Vec<&str> = vec![".."; sp.len() - common];
    out.extend(&pp[common..]);
    Ok(s(vm, if out.is_empty() { ".".into() } else { out.join("/") }))
}

fn p_commonprefix(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let items = crate::builtins::to_list_of_str(vm, &a[0])?;
    let Some(first) = items.first() else { return Ok(s(vm, String::new())) };
    let mut n = first.len();
    for it in &items[1..] {
        n = n.min(first.bytes().zip(it.bytes()).take_while(|(x, y)| x == y).count());
    }
    while !first.is_char_boundary(n) {
        n -= 1;
    }
    Ok(s(vm, first[..n].to_string()))
}

fn listdir(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let p = match a.first() {
        Some(v) => path_arg(vm, v)?,
        None => ".".into(),
    };
    match vm.host.list_dir(&p) {
        Some(names) => {
            let items = names.into_iter().map(|n| s(vm, n)).collect();
            Ok(vm.types.new_list(items))
        }
        None => Err(vm.exc_msg("FileNotFoundError", format!("[Errno 2] No such file or directory: '{p}'"))),
    }
}

fn getcwd(vm: &mut Vm, _a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(vm.types.new_str("/"))
}

fn remove(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let p = path_arg(vm, &a[0])?;
    if !vm.host.remove_file(&p) {
        return Err(vm.exc_msg("FileNotFoundError", format!("[Errno 2] No such file or directory: '{p}'")));
    }
    Ok(Value::None)
}

fn rename(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let x = path_arg(vm, &a[0])?;
    let y = path_arg(vm, &a[1])?;
    if !vm.host.rename(&x, &y) {
        return Err(vm.exc_msg("OSError", format!("cannot rename '{x}' to '{y}'")));
    }
    Ok(Value::None)
}

fn mkdir(vm: &mut Vm, a: &[Value], k: Kw) -> PyResult<Value> {
    let p = path_arg(vm, &a[0])?;
    let exist_ok = match k.iter().find(|(n, _)| &**n == "exist_ok") {
        Some((_, v)) => ops::truthy(vm, v)?,
        None => false,
    };
    if vm.host.stat(&p) == Some(true) {
        if exist_ok {
            return Ok(Value::None);
        }
        return Err(vm.exc_msg("FileExistsError", format!("[Errno 17] File exists: '{p}'")));
    }
    if !vm.host.make_dir(&p) {
        return Err(vm.exc_msg("PermissionError", format!("cannot create '{p}'")));
    }
    Ok(Value::None)
}

fn urandom(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    let n = ops::index_of(vm, &a[0])?.max(0) as usize;
    // Deterministic per call sequence is unacceptable for crypto but scripts
    // only use this for seeding; mix the clock through splitmix64.
    let mut x = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0x9e3779b97f4a7c15);
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        x = x.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = x;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        out.push((z ^ (z >> 31)) as u8);
    }
    Ok(vm.types.new_bytes(out))
}

fn getpid(_vm: &mut Vm, _a: &[Value], _k: Kw) -> PyResult<Value> {
    Ok(Value::Int(1))
}

fn fspath(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    if a[0].as_str().is_some() || ops::bytes_of(&a[0]).is_some() {
        return Ok(a[0].clone());
    }
    match ops::call_special(vm, &a[0], "__fspath__", Vec::new())? {
        Some(v) => Ok(v),
        None => Err(vm.type_error("expected str, bytes or os.PathLike object")),
    }
}

fn getenv(vm: &mut Vm, a: &[Value], _k: Kw) -> PyResult<Value> {
    if let Some(Value::Obj(m)) = crate::import::get_module(vm, "posix") {
        if let Some(env) = m.dict_get_str_raw("environ") {
            if let Some(Value::Obj(d)) = Some(env) {
                if let Some(v) = ops::dict_get(vm, &d, &a[0])? {
                    return Ok(v);
                }
            }
        }
    }
    Ok(a.get(1).cloned().unwrap_or(Value::None))
}

pub fn init(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = module_with(vm, "posix", &[
        ("listdir", listdir), ("getcwd", getcwd), ("getcwdu", getcwd), ("remove", remove), ("unlink", remove), ("rename", rename),
        ("replace", rename), ("mkdir", mkdir), ("makedirs", mkdir), ("urandom", urandom), ("getpid", getpid), ("fspath", fspath),
        ("getenv", getenv), ("_path_join", p_join), ("_path_split", p_split), ("_path_basename", p_basename),
        ("_path_dirname", p_dirname), ("_path_splitext", p_splitext), ("_path_normpath", p_normpath), ("_path_abspath", p_abspath),
        ("_path_isabs", p_isabs), ("_path_expanduser", p_expanduser), ("_path_exists", p_exists), ("_path_isfile", p_isfile),
        ("_path_isdir", p_isdir), ("_path_getsize", p_getsize), ("_path_getmtime", p_getmtime), ("_path_relpath", p_relpath),
        ("_path_commonprefix", p_commonprefix),
    ]);
    let env = vm.types.new_dict();
    set(vm, &m, "environ", env);
    for (n, v) in [("sep", "/"), ("altsep", ""), ("extsep", "."), ("pathsep", ":"), ("linesep", "\n"), ("curdir", "."), ("pardir", ".."), ("devnull", "/dev/null"), ("name", "posix")] {
        let sv = vm.types.new_str(v);
        set(vm, &m, n, sv);
    }
    set(vm, &m, "altsep", Value::None);
    let e = vm.types.exc("OSError");
    set(vm, &m, "error", Value::Obj(e));
    Ok(m)
}

pub fn init_errno(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = module_with(vm, "errno", &[]);
    for (n, v) in [("EPERM", 1), ("ENOENT", 2), ("EIO", 5), ("EBADF", 9), ("EAGAIN", 11), ("EACCES", 13), ("EEXIST", 17), ("ENOTDIR", 20), ("EISDIR", 21), ("EINVAL", 22), ("ENOSPC", 28), ("EROFS", 30), ("ENOTEMPTY", 39)] {
        set(vm, &m, n, Value::Int(v));
    }
    Ok(m)
}
