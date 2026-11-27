// SPDX-License-Identifier: MPL-2.0
//! `renpy.display.im`: image manipulators, after Ren'Py 8.3 `im.py` (MIT).
//!
//! Each manipulator describes a bounded image graph over the shared image
//! service (`ImageMetadata::begin_process`): file inputs, resize, flip,
//! colour matrices, channel lookups, blur, crop margins and source-over
//! composition. Plain files are drawn directly as image layers. Derived
//! images are processed on shared workers and published as client-scoped
//! image resources; until a result is ready the image renders transparent
//! and requests a redraw.
use super::{
    py::{self, *},
    render::{self, RenderData, TexKind, Texture, new_render, new_texture},
};
use crysirror_core::{
    image_processing::{ImageCompositeLayer, ImageCompositeMath, ImageInput, ImageJob, ImageProcessState, ImageResource},
    service::HostServices,
    state::ClientId,
};
use crysirror_python::{ObjRef, PyResult, Value, Vm, builtins::Kw, ops};
use serde_json::{Value as J, json};
use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::Arc};

const MODULE: &str = "renpy.display.im";

/// A bounded image graph.
#[derive(Clone, Debug)]
pub enum Graph {
    File { name: String, size: [u32; 2] },
    Effect { input: Box<Graph>, effects: Vec<J>, size: [u32; 2] },
    Composite { size: [u32; 2], layers: Vec<(Graph, i32, i32)> },
    Mask { base: Box<Graph>, mask: Box<Graph> },
    /// Encoded image bytes held by the game (`im.Data`, save screenshots).
    Encoded { name: String, bytes: Arc<[u8]>, size: [u32; 2] },
}

impl Graph {
    pub fn size(&self) -> [u32; 2] {
        match self {
            Graph::File { size, .. } | Graph::Effect { size, .. } | Graph::Composite { size, .. } => *size,
            Graph::Mask { base, .. } => base.size(),
            Graph::Encoded { size, .. } => *size,
        }
    }
}

enum State {
    Pending(ImageJob),
    Ready(Arc<dyn ImageResource>),
    /// Sent to the owner for publication under this request id.
    Publishing(u32),
    Published(String),
    Failed(String),
}

#[derive(Default)]
pub struct ImCache {
    entries: HashMap<String, (State, [u32; 2])>,
    next_id: u32,
}

fn cache() -> Rc<RefCell<ImCache>> {
    thread_local! {
        static CACHE: Rc<RefCell<ImCache>> = Rc::new(RefCell::new(ImCache::default()));
    }
    CACHE.with(|c| c.clone())
}

fn asset_input(vm: &Vm, name: &str, size: [u32; 2]) -> Result<ImageInput, String> {
    let path = super::asset_path(vm, name).ok_or_else(|| format!("Couldn't find file '{name}'."))?;
    Ok(ImageInput::Asset { path, size, fingerprint: None })
}

fn to_input(vm: &Vm, g: &Graph) -> Result<(ImageInput, Vec<J>), String> {
    Ok(match g {
        Graph::File { name, size } => (asset_input(vm, name, *size)?, Vec::new()),
        Graph::Encoded { name, bytes, size } => (ImageInput::Encoded { name: name.clone(), bytes: bytes.clone(), size: *size }, Vec::new()),
        Graph::Effect { input, effects, .. } => {
            let (inner, inner_fx) = to_input(vm, input)?;
            let inner = if inner_fx.is_empty() { inner } else { ImageInput::Process { input: Box::new(inner), effects: fx_bytes(&inner_fx)? } };
            (inner, effects.clone())
        }
        Graph::Composite { size, layers } => {
            let mut out = Vec::new();
            for (layer, x, y) in layers {
                let (inp, fx) = to_input(vm, layer)?;
                let inp = if fx.is_empty() { inp } else { ImageInput::Process { input: Box::new(inp), effects: fx_bytes(&fx)? } };
                let [w, h] = layer.size();
                out.push(ImageCompositeLayer {
                    input: Box::new(inp),
                    source: [0, 0, w as i32, h as i32],
                    destination: [*x, *y],
                    clip: None,
                    opacity: 255,
                    math: ImageCompositeMath::AlphaOnAlphaScalar256,
                });
            }
            (ImageInput::Composite { base: Box::new(ImageInput::Solid { size: *size, color: [0; 4] }), layers: out }, Vec::new())
        }
        Graph::Mask { base, mask } => {
            let (b, bf) = to_input(vm, base)?;
            let (m, mf) = to_input(vm, mask)?;
            let b = if bf.is_empty() { b } else { ImageInput::Process { input: Box::new(b), effects: fx_bytes(&bf)? } };
            let m = if mf.is_empty() { m } else { ImageInput::Process { input: Box::new(m), effects: fx_bytes(&mf)? } };
            (ImageInput::Raster { destination: Box::new(b), source: Box::new(m) }, vec![json!({"kind": "copy_channel", "source_channel": 0, "destination_channels": 8, "invert": false})])
        }
    })
}

fn fx_bytes(fx: &[J]) -> Result<Arc<[u8]>, String> {
    serde_json::to_vec(fx).map(Arc::from).map_err(|e| e.to_string())
}

/// Texture of a graph: the file itself, or the processed and published
/// result. `None` while the worker is still running.
pub fn texture_for(vm: &mut Vm, key: &str, g: &Graph) -> PyResult<Option<Texture>> {
    let [w, h] = g.size();
    if let Graph::File { name, .. } = g {
        return Ok(Some(Texture { kind: TexKind::Asset(name.clone()), width: w as f64, height: h as f64, crop: None, full: None, opaque: false }));
    }
    let c = cache();
    {
        let c = c.borrow();
        if let Some((state, size)) = c.entries.get(key) {
            return match state {
                State::Published(p) => Ok(Some(Texture { kind: TexKind::Published(p.clone()), width: size[0] as f64, height: size[1] as f64, crop: None, full: None, opaque: false })),
                State::Failed(e) => Err(py::error(vm, e.clone())),
                _ => Ok(None),
            };
        }
    }
    let images = super::host(vm).borrow().images.clone().ok_or_else(|| py::error(vm, "image service unavailable"))?;
    let (input, fx) = to_input(vm, g).map_err(|e| py::error(vm, e))?;
    let state = match fx_bytes(&fx).and_then(|b| images.begin_process(input, b).map_err(|e| e.to_string())) {
        Ok(job) => State::Pending(job),
        Err(e) => State::Failed(e),
    };
    c.borrow_mut().entries.insert(key.to_owned(), (state, [w, h]));
    Ok(None)
}

/// Script thread, end of a tick: polls the image jobs and hands finished
/// resources to the owner for publication.
pub fn take_publish() -> Vec<(u32, Arc<dyn ImageResource>)> {
    let c = cache();
    let mut c = c.borrow_mut();
    let mut next_id = c.next_id;
    let mut out = Vec::new();
    for (_, (state, size)) in c.entries.iter_mut() {
        if let State::Pending(job) = state {
            match job.poll() {
                Ok(ImageProcessState::Pending) => {}
                Ok(ImageProcessState::Ready(r)) => *state = State::Ready(r),
                Ok(ImageProcessState::Failed(e)) => *state = State::Failed(e),
                Err(e) => *state = State::Failed(e.to_string()),
            }
        }
        if let State::Ready(r) = state {
            next_id = next_id.wrapping_add(1) & 0xffff;
            *size = r.size();
            out.push((next_id, r.clone()));
            *state = State::Publishing(next_id);
        }
    }
    c.next_id = next_id;
    out
}

/// Script thread, start of a tick: records the owner's publication
/// results. True when an image became drawable.
pub fn apply_published(results: &[(u32, Result<String, String>)]) -> bool {
    if results.is_empty() {
        return false;
    }
    let c = cache();
    let mut c = c.borrow_mut();
    let mut changed = false;
    for (_, (state, _)) in c.entries.iter_mut() {
        if let State::Publishing(id) = state
            && let Some((_, r)) = results.iter().find(|(i, _)| i == id)
        {
            *state = match r {
                Ok(path) => State::Published(path.clone()),
                Err(e) => State::Failed(e.clone()),
            };
            changed = true;
        }
    }
    changed
}

/// Owner thread: publishes the resources a tick requested. Resource ids
/// are `0x5250_0000 + request id` within the client.
pub fn publish(host: &mut dyn HostServices, client: ClientId, requests: Vec<(u32, Arc<dyn ImageResource>)>) -> Vec<(u32, Result<String, String>)> {
    requests
        .into_iter()
        .map(|(id, r)| {
            let res = host.publish_image_resource(client, 0x5250_0000 + id, r).map(|u| u.path.relative().to_owned()).map_err(|e| e.to_string());
            (id, res)
        })
        .collect()
}

pub fn pending(vm: &Vm) -> bool {
    let _ = vm;
    cache().borrow().entries.values().any(|(s, _)| matches!(s, State::Pending(_) | State::Ready(_) | State::Publishing(_)))
}

pub fn clear() {
    cache().borrow_mut().entries.clear();
}

// Graph construction ---------------------------------------------------------------------

fn file_size(vm: &mut Vm, name: &str) -> PyResult<[u32; 2]> {
    super::image::image_size(vm, name).map_err(|e| py::error(vm, e))
}

fn attr_f(vm: &mut Vm, this: &Value, name: &str) -> PyResult<f64> {
    let v = py::get(vm, this, name)?;
    py::f64_of(vm, &v)
}

fn table(vm: &mut Vm, v: &Value) -> PyResult<Vec<u8>> {
    let b = match ops::bytes_of(v) {
        Some(b) => b.to_vec(),
        None => match v.as_str() {
            Some(s) => s.chars().map(|c| c as u32 as u8).collect(),
            None => py::items(vm, v)?.iter().map(|x| py::int(vm, x).map(|i| i as u8)).collect::<PyResult<Vec<u8>>>()?,
        },
    };
    if b.len() != 256 {
        return Err(py::error(vm, "im.Map tables must have 256 entries."));
    }
    Ok(b)
}

fn lookup(channel: u8, table: Vec<u8>) -> J {
    json!({"kind": "channel_lookup", "channel": channel, "write_mask": 1u8 << channel, "table": table})
}

/// Builds the graph of an ImageBase.
pub fn graph(vm: &mut Vm, d: &Value) -> PyResult<Graph> {
    let cls = d.obj().map(|o| o.class_name()).unwrap_or_default();
    let child = |vm: &mut Vm, name: &str| -> PyResult<Graph> {
        let c = py::get(vm, d, name)?;
        graph(vm, &c)
    };
    Ok(match cls.as_str() {
        "Image" => {
            let name = py::get(vm, d, "filename")?;
            let name = py::string(vm, &name)?;
            let size = file_size(vm, &name)?;
            Graph::File { name, size }
        }
        "UnoptimizedTexture" => child(vm, "image")?,
        "Scale" => {
            let input = child(vm, "image")?;
            let w = attr_f(vm, d, "width")?.max(1.) as u32;
            let h = attr_f(vm, d, "height")?.max(1.) as u32;
            let bilinear = { let b = py::get_or_none(vm, d, "bilinear")?; ops::truthy(vm, &b)? };
            let kind = if bilinear { "resize" } else { "resize_nearest" };
            Graph::Effect { input: Box::new(input), effects: vec![json!({"kind": kind, "width": w, "height": h})], size: [w, h] }
        }
        "FactorScale" => {
            let input = child(vm, "image")?;
            let [iw, ih] = input.size();
            let w = ((iw as f64 * attr_f(vm, d, "width")?) as u32).max(1);
            let h = ((ih as f64 * attr_f(vm, d, "height")?) as u32).max(1);
            let bilinear = { let b = py::get_or_none(vm, d, "bilinear")?; ops::truthy(vm, &b)? };
            let kind = if bilinear { "resize" } else { "resize_nearest" };
            Graph::Effect { input: Box::new(input), effects: vec![json!({"kind": kind, "width": w, "height": h})], size: [w, h] }
        }
        "Flip" => {
            let input = child(vm, "image")?;
            let hz = { let v = py::get_or_none(vm, d, "horizontal")?; ops::truthy(vm, &v)? };
            let vt = { let v = py::get_or_none(vm, d, "vertical")?; ops::truthy(vm, &v)? };
            let size = input.size();
            Graph::Effect { input: Box::new(input), effects: vec![json!({"kind": "flip", "horizontal": hz, "vertical": vt})], size }
        }
        "Rotozoom" => {
            let input = child(vm, "image")?;
            let angle = attr_f(vm, d, "angle")?;
            let zoom = attr_f(vm, d, "zoom")?;
            let quarter = angle.rem_euclid(90.) == 0.;
            if !quarter {
                return Err(py::error(vm, "im.Rotozoom with a non-right angle needs an affine image resample operation that the shared image service does not provide."));
            }
            let mut fx = Vec::new();
            let [mut w, mut h] = input.size();
            if zoom != 1. {
                w = ((w as f64 * zoom) as u32).max(1);
                h = ((h as f64 * zoom) as u32).max(1);
                fx.push(json!({"kind": "resize", "width": w, "height": h}));
            }
            // pygame rotates counter-clockwise; the shared operation turns clockwise.
            let cw = (360. - angle.rem_euclid(360.)).rem_euclid(360.) as u16;
            if cw != 0 {
                fx.push(json!({"kind": "rotate", "degrees": cw}));
                if cw % 180 != 0 {
                    std::mem::swap(&mut w, &mut h);
                }
            }
            Graph::Effect { input: Box::new(input), effects: fx, size: [w, h] }
        }
        "Crop" => {
            let input = child(vm, "image")?;
            let [iw, ih] = input.size();
            let x = attr_f(vm, d, "x")? as i32;
            let y = attr_f(vm, d, "y")? as i32;
            let w = attr_f(vm, d, "w")? as i32;
            let h = attr_f(vm, d, "h")? as i32;
            let margins = [-x, -y, x + w - iw as i32, y + h - ih as i32];
            Graph::Effect { input: Box::new(input), effects: vec![json!({"kind": "canvas_margins", "margins": margins, "fill": [0, 0, 0, 0]})], size: [w.max(1) as u32, h.max(1) as u32] }
        }
        "Map" => {
            let input = child(vm, "image")?;
            let mut fx = Vec::new();
            for (c, n) in ["rmap", "gmap", "bmap", "amap"].iter().enumerate() {
                let v = py::get(vm, d, n)?;
                fx.push(lookup(c as u8, table(vm, &v)?));
            }
            let size = input.size();
            Graph::Effect { input: Box::new(input), effects: fx, size }
        }
        "Recolor" => {
            let input = child(vm, "image")?;
            let mut fx = Vec::new();
            for (c, n) in ["rmul", "gmul", "bmul", "amul"].iter().enumerate() {
                let m = attr_f(vm, d, n)? as u32;
                let t: Vec<u8> = (0..256u32).map(|v| ((v * m) >> 8).min(255) as u8).collect();
                fx.push(lookup(c as u8, t));
            }
            let size = input.size();
            Graph::Effect { input: Box::new(input), effects: fx, size }
        }
        "Twocolor" => {
            let input = child(vm, "image")?;
            let white = py::get(vm, d, "white")?;
            let black = py::get(vm, d, "black")?;
            let w = super::color::rgba(vm, &white)?;
            let b = super::color::rgba(vm, &black)?;
            let mut fx = Vec::new();
            for c in 0..3 {
                let (lo, hi) = (b[c] as f64, w[c] as f64);
                let t: Vec<u8> = (0..256).map(|v| (lo + (hi - lo) * v as f64 / 255.) as u8).collect();
                fx.push(lookup(c as u8, t));
            }
            let t: Vec<u8> = (0..256u32).map(|v| ((v * (w[3] as u32 + 1)) >> 8) as u8).collect();
            fx.push(lookup(3, t));
            let size = input.size();
            Graph::Effect { input: Box::new(input), effects: fx, size }
        }
        "Blur" => {
            let input = child(vm, "image")?;
            let rx = attr_f(vm, d, "rx")?;
            let ry = attr_f(vm, d, "ry")?;
            let size = input.size();
            Graph::Effect { input: Box::new(input), effects: vec![json!({"kind": "gaussian_blur", "sigma": ((rx + ry) / 2.) as f32})], size }
        }
        "MatrixColor" => {
            let input = child(vm, "image")?;
            let m = py::get(vm, d, "matrix")?;
            let m = py::items(vm, &m)?;
            let mut mat = [[0f32; 5]; 4];
            for r in 0..4 {
                for c in 0..5 {
                    mat[r][c] = py::f64_of(vm, &m[r * 5 + c])? as f32;
                }
            }
            let size = input.size();
            Graph::Effect { input: Box::new(input), effects: vec![json!({"kind": "rgba_color_matrix", "spec": {"matrix": mat}})], size }
        }
        "Tile" => {
            let input = child(vm, "image")?;
            let sz = py::get_or_none(vm, d, "size")?;
            let (w, h) = if py::is_none(&sz) {
                (py::config_f64(vm, "screen_width")?, py::config_f64(vm, "screen_height")?)
            } else {
                let s = py::items(vm, &sz)?;
                (py::f64_of(vm, &s[0])?, py::f64_of(vm, &s[1])?)
            };
            let [sw, sh] = input.size();
            let mut layers = Vec::new();
            let mut y = 0;
            while y < h as i32 && sh > 0 {
                let mut x = 0;
                while x < w as i32 && sw > 0 {
                    layers.push((input.clone(), x, y));
                    x += sw as i32;
                }
                y += sh as i32;
            }
            if layers.len() > 4096 {
                return Err(py::error(vm, "im.Tile repeats its image too many times."));
            }
            Graph::Composite { size: [w as u32, h as u32], layers }
        }
        "AlphaMask" => {
            let base = child(vm, "base")?;
            let mask = child(vm, "mask")?;
            if base.size() != mask.size() {
                return Err(py::error(vm, "AlphaMask surfaces must be the same size."));
            }
            Graph::Mask { base: Box::new(base), mask: Box::new(mask) }
        }
        "Composite" => {
            let images = py::get(vm, d, "images")?;
            let images = py::items(vm, &images)?;
            let positions = py::get(vm, d, "positions")?;
            let positions = py::items(vm, &positions)?;
            let mut layers = Vec::new();
            for (im, pos) in images.iter().zip(positions.iter()) {
                let g = graph(vm, im)?;
                let p = py::items(vm, pos)?;
                layers.push((g, py::f64_of(vm, &p[0])? as i32, py::f64_of(vm, &p[1])? as i32));
            }
            let sz = py::get_or_none(vm, d, "size")?;
            let size = if py::is_none(&sz) {
                layers.first().map(|l| l.0.size()).unwrap_or([1, 1])
            } else {
                let s = py::items(vm, &sz)?;
                [py::f64_of(vm, &s[0])? as u32, py::f64_of(vm, &s[1])? as u32]
            };
            Graph::Composite { size, layers }
        }
        "Data" => {
            let data = py::get(vm, d, "data")?;
            let bytes = match data.payload() {
                Some(crysirror_python::Payload::Bytes(b)) => b.to_vec(),
                _ => return Err(py::error(vm, "im.Data expects bytes.")),
            };
            let f = py::get(vm, d, "filename")?;
            let f = py::string(vm, &f)?;
            encoded_graph(vm, &f, bytes)?
        }
        "ZipFileImage" => {
            let z = py::get(vm, d, "zipfilename")?;
            let z = py::string(vm, &z)?;
            let f = py::get(vm, d, "filename")?;
            let f = py::string(vm, &f)?;
            let st = super::savestore::store(vm);
            let Some(member) = st.read(&z).and_then(|b| super::savestore::zip_member(&b, &f)) else {
                return Err(py::error(vm, format!("Couldn't read {f} from {z}.")));
            };
            encoded_graph(vm, &f, member)?
        }
        _ => return Err(py::error(vm, format!("{cls}.load() returns a pygame surface; CrySirror image manipulators are described as shared image graphs instead."))),
    })
}

/// An encoded-bytes graph; dimensions come from the header.
fn encoded_graph(vm: &mut Vm, name: &str, bytes: Vec<u8>) -> PyResult<Graph> {
    let Some(size) = crysirror_image::encoded_dimensions_hint(&bytes) else {
        return Err(py::error(vm, format!("Couldn't determine the size of image {name}.")));
    };
    Ok(Graph::Encoded { name: name.to_owned(), bytes: bytes.into(), size })
}

/// PNG of straight RGBA8 pixels (`load_rgba`), for the shared decoder.
pub fn png_rgba(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    use std::io::Write as _;
    fn crc(data: &[u8]) -> u32 {
        let mut c = !0u32;
        for &x in data {
            c ^= x as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { (c >> 1) ^ 0xedb8_8320 } else { c >> 1 };
            }
        }
        !c
    }
    let chunk = |out: &mut Vec<u8>, kind: &[u8], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut c = kind.to_vec();
        c.extend_from_slice(data);
        out.extend_from_slice(&c);
        out.extend_from_slice(&crc(&c).to_be_bytes());
    };
    let mut raw = Vec::with_capacity((w as usize * 4 + 1) * h as usize);
    for row in rgba.chunks(w as usize * 4).take(h as usize) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    let _ = z.write_all(&raw);
    let idat = z.finish().unwrap_or_default();
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &idat);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// `load_rgba(data, size)`: a texture of RGBA8888 bytes.
fn f_load_rgba(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let data = match args[0].payload() {
        Some(crysirror_python::Payload::Bytes(b)) => b.to_vec(),
        _ => return Err(vm.type_error("load_rgba expects bytes")),
    };
    let s = py::items(vm, &args[1])?;
    let (w, h) = (py::int(vm, &s[0])? as u32, py::int(vm, &s[1])? as u32);
    if data.len() < (w as usize) * (h as usize) * 4 {
        return Err(vm.exc_msg("ValueError", "load_rgba data is smaller than the given size"));
    }
    let png = png_rgba(w, h, &data);
    let key = format!("rgba:{}", crysirror_resource::checksum::md5_legacy(&png).iter().map(|b| format!("{b:02x}")).collect::<String>());
    let g = Graph::Encoded { name: key.clone(), bytes: png.into(), size: [w, h] };
    match texture_for(vm, &key, &g)? {
        Some(t) => Ok(new_texture(vm, t)),
        None => Ok(new_texture(vm, Texture { kind: TexKind::Solid([0; 4]), width: w as f64, height: h as f64, crop: None, full: None, opaque: false })),
    }
}

fn identity_key(vm: &mut Vm, d: &Value) -> PyResult<String> {
    let ident = py::get(vm, d, "identity")?;
    Ok(ops::repr(vm, &ident)?.to_string())
}

fn ib_render(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    match ib_render_inner(vm, this) {
        Ok(r) => Ok(r),
        Err(e) => {
            if py::config_bool(vm, "raise_image_load_exceptions").unwrap_or(true) {
                return Err(e);
            }
            let msg = match &e {
                crysirror_python::PyErr::Exc(o) => ops::str(vm, &Value::Obj(o.clone())).map(|s| s.to_string()).unwrap_or_default(),
                _ => "image load failed".into(),
            };
            let text = py::import_attr(vm, "renpy.text.text", "Text")?;
            let m = vm.types.new_string(msg);
            let fail = vm.call(&text, vec![m], vec![("style".into(), vm.types.new_str("_image_error"))])?;
            py::set(vm, this, "fail", fail.clone())?;
            vm.call_method(&fail, "render", args[1..].to_vec())
        }
    }
}

fn ib_render_inner(vm: &mut Vm, this: &Value) -> PyResult<Value> {
    let g = graph(vm, this)?;
    let key = identity_key(vm, this)?;
    let oversample = { let v = py::get_or_none(vm, this, "oversample")?; if py::is_none(&v) { 1. } else { py::f64_of(vm, &v)? } };
    let [w, h] = g.size();
    let (vw, vh) = (w as f64 / oversample, h as f64 / oversample);
    let mut rd = RenderData::new(vw, vh);
    match texture_for(vm, &key, &g)? {
        Some(mut t) => {
            t.width = w as f64;
            t.height = h as f64;
            let tv = new_texture(vm, t);
            let source = render::source_of(vm, &tv)?;
            rd.children.push(render::Blit { source, x: 0., y: 0., focus: false, main: true });
            if oversample != 1. {
                let s = 1. / oversample;
                rd.reverse = Some(render::Mat::scale(s, s, 1.));
                rd.forward = Some(render::Mat::scale(oversample, oversample, 1.));
            }
        }
        None => {
            render::redraw(vm, this, 0.);
        }
    }
    Ok(new_render(vm, rd))
}

fn ib_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let mut props: Vec<_> = Vec::new();
    let mut extra = |k: &str| -> Option<Value> { kw.iter().find(|(n, _)| &**n == k).map(|(_, v)| v.clone()) };
    let rle = extra("rle").unwrap_or(Value::None);
    let cachev = extra("cache").unwrap_or(Value::Bool(true));
    let ob = extra("optimize_bounds").unwrap_or(Value::Bool(true));
    let os = extra("oversample").unwrap_or(Value::Int(1));
    for (k, v) in kw {
        if !matches!(&**k, "rle" | "cache" | "optimize_bounds" | "oversample") {
            props.push((k.clone(), v.clone()));
        }
    }
    py::set(vm, this, "rle", rle)?;
    py::set(vm, this, "cache", cachev)?;
    py::set(vm, this, "optimize_bounds", ob)?;
    if py::f64_of(vm, &os)? <= 0. {
        return Err(py::error(vm, "Image's oversample parameter must be greater than 0."));
    }
    py::set(vm, this, "oversample", os)?;
    if !props.iter().any(|(k, _)| &**k == "style") {
        props.push(("style".into(), vm.types.new_str("image")));
    }
    super::core::d_init(vm, &[this.clone()], &props)?;
    let cls = this.obj().map(|o| o.class_name()).unwrap_or_default();
    let mut ident = vec![vm.types.new_string(cls)];
    ident.extend(args[1..].iter().cloned());
    let t = vm.types.new_tuple(ident);
    py::set(vm, this, "identity", t)?;
    Ok(Value::None)
}

fn ib_hash(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let i = py::get(vm, &args[0], "identity")?;
    Ok(Value::Int(ops::hash(vm, &i)?))
}

fn ib_eq(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let other = args.get(1).cloned().unwrap_or(Value::None);
    let base = py::import_attr(vm, MODULE, "ImageBase")?;
    if !base.obj().is_some_and(|b| ops::isinstance(vm, &other, b)) {
        return Ok(Value::Bool(false));
    }
    let a = py::get(vm, &args[0], "identity")?;
    let b = py::get(vm, &other, "identity")?;
    Ok(Value::Bool(ops::eq(vm, &a, &b)?))
}

fn ib_load(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Err(py::error(vm, "load method not implemented."))
}

fn ib_get_placement(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let fail = py::get_or_none(vm, &args[0], "fail")?;
    if !py::is_none(&fail) {
        return vm.call_method(&fail, "get_placement", Vec::new());
    }
    let s = py::get(vm, &args[0], "style")?;
    vm.call_method(&s, "get_placement", Vec::new())
}

fn ib_predict_one(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    // Start the worker early; prediction never blocks.
    if let Ok(g) = graph(vm, &args[0]) {
        let key = identity_key(vm, &args[0])?;
        let _ = texture_for(vm, &key, &g);
    }
    Ok(Value::None)
}

fn ib_predict_files(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    fn files(g: &Graph, out: &mut Vec<String>) {
        match g {
            Graph::File { name, .. } => out.push(name.clone()),
            Graph::Effect { input, .. } => files(input, out),
            Graph::Composite { layers, .. } => layers.iter().for_each(|l| files(&l.0, out)),
            Graph::Mask { base, mask } => {
                files(base, out);
                files(mask, out);
            }
            Graph::Encoded { .. } => {}
        }
    }
    let mut out = Vec::new();
    if let Ok(g) = graph(vm, &args[0]) {
        files(&g, &mut out);
    }
    let l = out.iter().map(|s| vm.types.new_str(s)).collect();
    Ok(vm.types.new_list(l))
}

fn ib_zero(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Int(0))
}

fn ib_get_oversample(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::get_or_none(vm, &args[0], "oversample")
}

fn ib_after_upgrade(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let v = py::int(vm, &args.get(1).cloned().unwrap_or(Value::Int(1)))?;
    if v < 1 {
        py::set(vm, &args[0], "cache", Value::Bool(true))?;
    }
    Ok(Value::None)
}

fn ib_child_hash(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = py::get(vm, &args[0], "image")?;
    vm.call_method(&c, "get_hash", Vec::new())
}

fn image_arg(vm: &mut Vm, v: &Value) -> PyResult<Value> {
    let f = py::import_attr(vm, MODULE, "image")?;
    vm.call(&f, vec![v.clone()], Vec::new())
}

/// Helper for manipulators taking `(im, ...)`: run ImageBase.__init__ with
/// all positional args, then set named attributes.
fn manip_init(vm: &mut Vm, args: &[Value], kw: Kw, names: &[(&str, Option<Value>)], props_keys: &[&str]) -> PyResult<Vec<Value>> {
    let this = &args[0];
    let mut vals: Vec<Value> = names.iter().map(|(_, d)| d.clone().unwrap_or(Value::None)).collect();
    let mut given = vec![false; names.len()];
    for (i, v) in args[1..].iter().enumerate().take(names.len()) {
        vals[i] = v.clone();
        given[i] = true;
    }
    let mut props = Vec::new();
    for (k, v) in kw {
        match names.iter().position(|(n, _)| *n == &**k) {
            Some(i) => {
                vals[i] = v.clone();
                given[i] = true;
            }
            None => props.push((k.clone(), v.clone())),
        }
    }
    for (i, (n, d)) in names.iter().enumerate() {
        if !given[i] && d.is_none() {
            return Err(vm.type_error(format!("missing argument {n}")));
        }
    }
    let _ = props_keys;
    if !vals.is_empty() && names[0].0 == "im" {
        vals[0] = image_arg(vm, &vals[0])?;
    }
    let mut init_args = vec![this.clone()];
    init_args.extend(vals.iter().cloned());
    ib_init(vm, &init_args, &props)?;
    if names.first().is_some_and(|n| n.0 == "im") {
        py::set(vm, this, "image", vals[0].clone())?;
        let os = vm.call_method(&vals[0], "get_oversample", Vec::new())?;
        py::set(vm, this, "oversample", os)?;
    }
    Ok(vals)
}

fn image_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let filename = arg_or(args, kw, 1, "filename", Value::None);
    let dpi = arg_or(args, kw, 2, "dpi", Value::Int(96));
    let fname = py::string(vm, &filename)?;
    let mut props: Vec<_> = kw.iter().filter(|(k, _)| !matches!(&**k, "filename" | "dpi")).cloned().collect();
    if fname.contains('@') {
        let base = fname.rsplit_once('.').map(|(b, _)| b).unwrap_or(&fname);
        let extras = base.rsplit_once('@').map(|(_, e)| e).unwrap_or("");
        let extras = extras.split('/').next().unwrap_or("");
        for i in extras.split(',') {
            match i.parse::<f64>() {
                Ok(v) => {
                    if !props.iter().any(|(k, _)| &**k == "oversample") {
                        props.push(("oversample".into(), Value::Float(v)));
                    }
                }
                Err(_) => return Err(py::error(vm, format!("Unknown image modifier {i:?} in {fname:?}."))),
            }
        }
    }
    ib_init(vm, &[this.clone(), filename.clone()], &props)?;
    py::set(vm, this, "filename", filename)?;
    py::set(vm, this, "dpi", dpi)?;
    let svg = fname.to_ascii_lowercase().ends_with(".svg");
    py::set(vm, this, "is_svg", Value::Bool(svg))?;
    py::set(vm, this, "pixel_perfect", Value::Bool(svg))?;
    Ok(Value::None)
}

fn image_repr_info(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let f = py::get(vm, &args[0], "filename")?;
    let r = ops::repr(vm, &f)?;
    Ok(vm.types.new_string(r.to_string()))
}

fn image_get_hash(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let f = py::get(vm, &args[0], "filename")?;
    Ok(Value::Int(ops::hash(vm, &f)?))
}

fn data_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let data = arg_or(args, kw, 1, "data", Value::None);
    let filename = arg_or(args, kw, 2, "filename", Value::None);
    let props: Vec<_> = kw.iter().filter(|(k, _)| !matches!(&**k, "data" | "filename")).cloned().collect();
    ib_init(vm, &[this.clone(), data.clone(), filename.clone()], &props)?;
    py::set(vm, this, "data", data)?;
    py::set(vm, this, "filename", filename)?;
    Ok(Value::None)
}

fn zip_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let z = arg_or(args, kw, 1, "zipfilename", Value::None);
    let f = arg_or(args, kw, 2, "filename", Value::None);
    let m = arg_or(args, kw, 3, "mtime", Value::Int(0));
    let props: Vec<_> = kw.iter().filter(|(k, _)| !matches!(&**k, "zipfilename" | "filename" | "mtime")).cloned().collect();
    ib_init(vm, &[this.clone(), z.clone(), f.clone(), m], &props)?;
    py::set(vm, this, "zipfilename", z)?;
    py::set(vm, this, "filename", f)?;
    Ok(Value::None)
}

fn composite_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let size = args.get(1).cloned().unwrap_or(Value::None);
    let rest = args.get(2..).unwrap_or(&[]).to_vec();
    let mut iargs = vec![this.clone(), size.clone()];
    iargs.extend(rest.iter().cloned());
    ib_init(vm, &iargs, kw)?;
    if rest.len() % 2 != 0 {
        return Err(py::error(vm, "Composite requires an odd number of arguments."));
    }
    py::set(vm, this, "size", size)?;
    let positions: Vec<Value> = rest.iter().step_by(2).cloned().collect();
    let mut images = Vec::new();
    for i in rest.iter().skip(1).step_by(2) {
        images.push(image_arg(vm, i)?);
    }
    if let Some(first) = images.first() {
        let os = vm.call_method(first, "get_oversample", Vec::new())?;
        py::set(vm, this, "oversample", os)?;
    }
    py::set(vm, this, "positions", vm.types.new_tuple(positions))?;
    py::set(vm, this, "images", vm.types.new_list(images))?;
    Ok(Value::None)
}

fn composite_get_hash(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let images = py::get(vm, &args[0], "images")?;
    let mut rv = 0i64;
    for i in py::items(vm, &images)? {
        let h = vm.call_method(&i, "get_hash", Vec::new())?;
        rv = rv.wrapping_add(py::int(vm, &h)?);
    }
    Ok(Value::Int(rv))
}

fn scale_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let v = manip_init(vm, args, kw, &[("im", None), ("width", None), ("height", None), ("bilinear", Some(Value::Bool(true)))], &[])?;
    let w = py::int(vm, &v[1])?;
    let h = py::int(vm, &v[2])?;
    py::set(vm, &args[0], "width", Value::Int(w))?;
    py::set(vm, &args[0], "height", Value::Int(h))?;
    py::set(vm, &args[0], "bilinear", v[3].clone())?;
    Ok(Value::None)
}

fn factorscale_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut a = args.to_vec();
    let mut k: Vec<_> = kw.to_vec();
    // height defaults to width.
    let has_h = a.len() > 3 || k.iter().any(|(n, _)| &**n == "height");
    if !has_h {
        let w = a.get(2).cloned().or_else(|| k.iter().find(|(n, _)| &**n == "width").map(|(_, v)| v.clone())).unwrap_or(Value::Int(1));
        if a.len() == 3 {
            a.push(w);
        } else {
            k.push(("height".into(), w));
        }
    }
    let v = manip_init(vm, &a, &k, &[("im", None), ("width", None), ("height", None), ("bilinear", Some(Value::Bool(true)))], &[])?;
    py::set(vm, &args[0], "width", v[1].clone())?;
    py::set(vm, &args[0], "height", v[2].clone())?;
    py::set(vm, &args[0], "bilinear", v[3].clone())?;
    Ok(Value::None)
}

fn flip_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let v = manip_init(vm, args, kw, &[("im", None), ("horizontal", Some(Value::Bool(false))), ("vertical", Some(Value::Bool(false)))], &[])?;
    if !ops::truthy(vm, &v[1])? && !ops::truthy(vm, &v[2])? {
        return Err(py::error(vm, "im.Flip must be called with a true value for horizontal or vertical."));
    }
    py::set(vm, &args[0], "horizontal", v[1].clone())?;
    py::set(vm, &args[0], "vertical", v[2].clone())?;
    Ok(Value::None)
}

fn rotozoom_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let v = manip_init(vm, args, kw, &[("im", None), ("angle", None), ("zoom", None)], &[])?;
    py::set(vm, &args[0], "angle", v[1].clone())?;
    py::set(vm, &args[0], "zoom", v[2].clone())?;
    Ok(Value::None)
}

fn crop_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut a = args.to_vec();
    let y_given = a.len() > 3 || kw.iter().any(|(k, _)| &**k == "y");
    if !y_given {
        let x = a.get(2).cloned().or_else(|| kw.iter().find(|(k, _)| &**k == "x").map(|(_, v)| v.clone())).unwrap_or(Value::None);
        let r = py::items(vm, &x)?;
        a.truncate(2);
        a.extend(r);
    }
    let k: Vec<_> = if y_given { kw.to_vec() } else { kw.iter().filter(|(n, _)| &**n != "x").cloned().collect() };
    let v = manip_init(vm, &a, &k, &[("im", None), ("x", None), ("y", None), ("w", None), ("h", None)], &[])?;
    for (i, n) in ["x", "y", "w", "h"].iter().enumerate() {
        py::set(vm, &args[0], n, v[i + 1].clone())?;
    }
    Ok(Value::None)
}

fn identity_ramp(vm: &Vm) -> Value {
    vm.types.new_bytes((0..=255u8).collect())
}

fn map_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let id = identity_ramp(vm);
    let v = manip_init(vm, args, kw, &[("im", None), ("rmap", Some(id.clone())), ("gmap", Some(id.clone())), ("bmap", Some(id.clone())), ("amap", Some(id)), ("force_alpha", Some(Value::Bool(false)))], &[])?;
    for (i, n) in ["rmap", "gmap", "bmap", "amap", "force_alpha"].iter().enumerate() {
        py::set(vm, &args[0], n, v[i + 1].clone())?;
    }
    Ok(Value::None)
}

fn twocolor_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut a = args.to_vec();
    let color = py::import_attr(vm, "renpy.easy", "color")?;
    for i in [2usize, 3] {
        if let Some(v) = a.get(i).cloned() {
            a[i] = vm.call(&color, vec![v], Vec::new())?;
        }
    }
    let mut k: Vec<_> = kw.to_vec();
    for (n, v) in k.iter_mut() {
        if matches!(&**n, "white" | "black") {
            *v = vm.call(&color, vec![v.clone()], Vec::new())?;
        }
    }
    let v = manip_init(vm, &a, &k, &[("im", None), ("white", None), ("black", None), ("force_alpha", Some(Value::Bool(false)))], &[])?;
    py::set(vm, &args[0], "white", v[1].clone())?;
    py::set(vm, &args[0], "black", v[2].clone())?;
    py::set(vm, &args[0], "force_alpha", v[3].clone())?;
    Ok(Value::None)
}

fn recolor_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let d = Some(Value::Int(255));
    let v = manip_init(vm, args, kw, &[("im", None), ("rmul", d.clone()), ("gmul", d.clone()), ("bmul", d.clone()), ("amul", d), ("force_alpha", Some(Value::Bool(false)))], &[])?;
    for (i, n) in ["rmul", "gmul", "bmul", "amul"].iter().enumerate() {
        let m = py::int(vm, &v[i + 1])? + 1;
        py::set(vm, &args[0], n, Value::Int(m))?;
    }
    py::set(vm, &args[0], "force_alpha", v[5].clone())?;
    Ok(Value::None)
}

fn blur_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let v = manip_init(vm, args, kw, &[("im", None), ("xrad", None), ("yrad", Some(Value::None))], &[])?;
    py::set(vm, &args[0], "rx", v[1].clone())?;
    let ry = if py::is_none(&v[2]) { v[1].clone() } else { v[2].clone() };
    py::set(vm, &args[0], "ry", ry)?;
    Ok(Value::None)
}

fn matrixcolor_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let mut a = args.to_vec();
    if let Some(m) = a.get(2).cloned() {
        let items = py::items(vm, &m)?;
        if items.len() != 20 && items.len() != 25 {
            return Err(py::error(vm, format!("ColorMatrix expects a 20 or 25 element matrix, got {} elements.", items.len())));
        }
        a[2] = vm.types.new_tuple(items);
    }
    let v = manip_init(vm, &a, kw, &[("im", None), ("matrix", None)], &[])?;
    py::set(vm, &args[0], "matrix", v[1].clone())?;
    Ok(Value::None)
}

fn tile_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let v = manip_init(vm, args, kw, &[("im", None), ("size", Some(Value::None))], &[])?;
    py::set(vm, &args[0], "size", v[1].clone())?;
    Ok(Value::None)
}

fn alphamask_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let base = arg_or(args, kw, 1, "base", Value::None);
    let mask = arg_or(args, kw, 2, "mask", Value::None);
    let props: Vec<_> = kw.iter().filter(|(k, _)| !matches!(&**k, "base" | "mask")).cloned().collect();
    ib_init(vm, &[this.clone(), base.clone(), mask.clone()], &props)?;
    let b = image_arg(vm, &base)?;
    let m = image_arg(vm, &mask)?;
    let os = vm.call_method(&b, "get_oversample", Vec::new())?;
    py::set(vm, this, "base", b)?;
    py::set(vm, this, "mask", m)?;
    py::set(vm, this, "oversample", os)?;
    Ok(Value::None)
}

fn alphamask_get_hash(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let b = py::get(vm, &args[0], "base")?;
    let m = py::get(vm, &args[0], "mask")?;
    let hb = vm.call_method(&b, "get_hash", Vec::new())?;
    let hm = vm.call_method(&m, "get_hash", Vec::new())?;
    Ok(Value::Int(py::int(vm, &hb)?.wrapping_add(py::int(vm, &hm)?)))
}

fn unopt_init(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let this = &args[0];
    let im = arg_or(args, kw, 1, "im", Value::None);
    let mut props: Vec<_> = kw.iter().filter(|(k, _)| &**k != "im").cloned().collect();
    props.push(("optimize_bounds".into(), Value::Bool(false)));
    ib_init(vm, &[this.clone(), im.clone()], &props)?;
    let i = image_arg(vm, &im)?;
    py::set(vm, this, "image", i)?;
    Ok(Value::None)
}

fn f_image(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let a = arg_or(args, kw, 0, "arg", Value::None);
    let loose = match py::kw(kw, "loose") { Some(v) => ops::truthy(vm, v)?, None => false };
    let props: Vec<_> = kw.iter().filter(|(k, _)| !matches!(&**k, "arg" | "loose")).cloned().collect();
    let base = py::import_attr(vm, MODULE, "ImageBase")?;
    if base.obj().is_some_and(|b| ops::isinstance(vm, &a, b)) {
        return Ok(a);
    }
    if a.as_str().is_some() {
        let img = py::import_attr(vm, MODULE, "Image")?;
        return vm.call(&img, vec![a], props);
    }
    if a.obj().is_some_and(|o| o.class_name() == "ImageReference") {
        vm.call_method(&a, "find_target", Vec::new())?;
        let t = py::get(vm, &a, "target")?;
        let mut k = props;
        k.push(("loose".into(), Value::Bool(loose)));
        return f_image(vm, &[t], &k);
    }
    if let Some(crysirror_python::Payload::Tuple(items)) = a.payload() {
        let mut params = vec![Value::None];
        for i in items.iter() {
            params.push(vm.types.new_tuple(vec![Value::Int(0), Value::Int(0)]));
            params.push(i.clone());
        }
        let c = py::import_attr(vm, MODULE, "Composite")?;
        return vm.call(&c, params, Vec::new());
    }
    if loose {
        return Ok(a);
    }
    if super::core::is_displayable(vm, &a) {
        return Err(py::error(vm, "Expected an image, but got a general displayable."));
    }
    Err(py::error(vm, "Could not construct image from argument."))
}

fn f_ramp(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = py::f64_of(vm, &args.first().cloned().unwrap_or(Value::Int(0)))?;
    let e = py::f64_of(vm, &args.get(1).cloned().unwrap_or(Value::Int(255)))?;
    let b: Vec<u8> = (0..256).map(|i| { let i = i as f64 / 255.; (e * i + s * (1. - i)) as u8 }).collect();
    Ok(vm.types.new_bytes(b))
}

fn matrix_class(vm: &mut Vm) -> PyResult<Value> {
    py::import_attr(vm, MODULE, "matrix")
}

fn f_grayscale(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let im = arg_or(args, kw, 0, "im", Value::None);
    let desat = py::kw(kw, "desat").cloned().or_else(|| args.get(1).cloned());
    let props: Vec<_> = kw.iter().filter(|(k, _)| !matches!(&**k, "im" | "desat")).cloned().collect();
    let m = matrix_class(vm)?;
    let sat = py::get(vm, &m, "saturation")?;
    let mut sargs = vec![Value::Float(0.)];
    if let Some(d) = desat {
        sargs.push(d);
    }
    let mx = vm.call(&sat, sargs, Vec::new())?;
    let mc = py::import_attr(vm, MODULE, "MatrixColor")?;
    vm.call(&mc, vec![im, mx], props)
}

fn f_sepia(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let im = arg_or(args, kw, 0, "im", Value::None);
    let tint = arg_or(args, kw, 1, "tint", Value::None);
    let desat = py::kw(kw, "desat").cloned().or_else(|| args.get(2).cloned());
    let props: Vec<_> = kw.iter().filter(|(k, _)| !matches!(&**k, "im" | "desat" | "tint")).cloned().collect();
    let m = matrix_class(vm)?;
    let sat = py::get(vm, &m, "saturation")?;
    let mut sargs = vec![Value::Float(0.)];
    if let Some(d) = desat {
        sargs.push(d);
    }
    let s = vm.call(&sat, sargs, Vec::new())?;
    let t = if py::is_none(&tint) { vm.types.new_tuple(vec![Value::Float(1.), Value::Float(0.94), Value::Float(0.76)]) } else { tint };
    let tf = py::get(vm, &m, "tint")?;
    let ti = py::items(vm, &t)?;
    let tm = vm.call(&tf, ti, Vec::new())?;
    let prod = ops::binary_op(vm, &s, &tm, crysirror_python::ast::BinOp::Mult, false, false)?;
    let mc = py::import_attr(vm, MODULE, "MatrixColor")?;
    vm.call(&mc, vec![im, prod], props)
}

fn f_color(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let im = arg_or(args, kw, 0, "im", Value::None);
    let color = arg_or(args, kw, 1, "color", Value::None);
    let c = super::color::rgba(vm, &color)?;
    let rc = py::import_attr(vm, MODULE, "Recolor")?;
    vm.call(&rc, vec![im, Value::Int(c[0] as i64), Value::Int(c[1] as i64), Value::Int(c[2] as i64), Value::Int(c[3] as i64)], Vec::new())
}

fn f_alpha(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let im = arg_or(args, kw, 0, "image", Value::None);
    let a = py::f64_of(vm, &arg_or(args, kw, 1, "alpha", Value::Float(1.)))?;
    let mut props: Vec<_> = kw.iter().filter(|(k, _)| !matches!(&**k, "image" | "alpha")).cloned().collect();
    props.push(("force_alpha".into(), Value::Bool(true)));
    let rc = py::import_attr(vm, MODULE, "Recolor")?;
    vm.call(&rc, vec![im, Value::Int(255), Value::Int(255), Value::Int(255), Value::Int((255. * a) as i64)], props)
}

fn f_load_image(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let d = f_image(vm, args, kw)?;
    let g = graph(vm, &d)?;
    let key = identity_key(vm, &d)?;
    match texture_for(vm, &key, &g)? {
        Some(t) => Ok(new_texture(vm, t)),
        None => {
            let [w, h] = g.size();
            Ok(new_texture(vm, Texture { kind: TexKind::Solid([0; 4]), width: w as f64, height: h as f64, crop: None, full: None, opaque: false }))
        }
    }
}

fn f_unoptimized_texture(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let d = args.first().cloned().unwrap_or(Value::None);
    let base = py::import_attr(vm, MODULE, "ImageBase")?;
    if base.obj().is_some_and(|b| ops::isinstance(vm, &d, b)) {
        let c = py::import_attr(vm, MODULE, "UnoptimizedTexture")?;
        return vm.call(&c, vec![d], Vec::new());
    }
    Ok(d)
}

fn f_render_for_texture(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let d = args.first().cloned().unwrap_or(Value::None);
    let g = |i: usize, vm: &mut Vm| -> PyResult<f64> { py::f64_of(vm, &args.get(i).cloned().unwrap_or(Value::Int(0))) };
    let (w, h, st, at) = (g(1, vm)?, g(2, vm)?, g(3, vm)?, g(4, vm)?);
    render::render(vm, &d, w, h, st, at)
}

// im.matrix -------------------------------------------------------------------------------

fn mat_items(vm: &mut Vm, v: &Value) -> PyResult<[f64; 25]> {
    let i = py::items(vm, v)?;
    let mut m = [0.; 25];
    match i.len() {
        25 => {
            for (k, x) in i.iter().enumerate() {
                m[k] = py::f64_of(vm, x)?;
            }
        }
        20 => {
            for (k, x) in i.iter().enumerate() {
                m[k] = py::f64_of(vm, x)?;
            }
            m[24] = 1.;
        }
        n => return Err(py::error(vm, format!("Matrix expects to be given 20 or 25 entries, not {n}."))),
    }
    Ok(m)
}

fn mat_value(vm: &mut Vm, m: [f64; 25]) -> PyResult<Value> {
    let c = matrix_class(vm)?;
    let l = vm.types.new_list(m.iter().map(|v| Value::Float(*v)).collect());
    vm.call(&c, vec![l], Vec::new())
}

fn mat_new(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let cls = args[0].obj().cloned().ok_or_else(|| vm.type_error("matrix.__new__"))?;
    let rest = &args[1..];
    let src = if rest.len() == 1 { rest[0].clone() } else { vm.types.new_tuple(rest.to_vec()) };
    let mut items = py::items(vm, &src)?;
    if items.len() == 20 {
        items.extend([Value::Int(0), Value::Int(0), Value::Int(0), Value::Int(0), Value::Int(1)]);
    }
    if items.len() != 25 {
        return Err(py::error(vm, format!("Matrix expects to be given 20 or 25 entries, not {}.", items.len())));
    }
    let tn = py::get(vm, &Value::Obj(vm.types.tuple.clone()), "__new__")?;
    let t = vm.types.new_tuple(items);
    vm.call(&tn, vec![Value::Obj(cls), t], Vec::new())
}

fn mat_mul_raw(a: &[f64; 25], b: &[f64; 25]) -> [f64; 25] {
    let mut r = [0.; 25];
    for y in 0..5 {
        for x in 0..5 {
            for i in 0..5 {
                r[x + y * 5] += a[x + i * 5] * b[i + y * 5];
            }
        }
    }
    r
}

fn is_number(v: &Value) -> bool {
    matches!(v, Value::Int(_) | Value::Float(_) | Value::Bool(_))
}

fn mat_mul(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let a = mat_items(vm, &args[0])?;
    let o = &args[1];
    if is_number(o) {
        let f = py::f64_of(vm, o)?;
        return mat_value(vm, a.map(|v| v * f));
    }
    if o.obj().is_some_and(|x| x.class_name() == "Color") {
        return Ok(vm.types.not_implemented.clone());
    }
    let b = mat_items(vm, o)?;
    mat_value(vm, mat_mul_raw(&a, &b))
}

fn mat_rmul(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let a = mat_items(vm, &args[0])?;
    let o = &args[1];
    if is_number(o) {
        let f = py::f64_of(vm, o)?;
        return mat_value(vm, a.map(|v| v * f));
    }
    let b = mat_items(vm, o)?;
    mat_value(vm, mat_mul_raw(&b, &a))
}

fn mat_add(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let a = mat_items(vm, &args[0])?;
    let o = &args[1];
    if is_number(o) {
        let f = py::f64_of(vm, o)?;
        return mat_value(vm, a.map(|v| v + f));
    }
    let b = mat_items(vm, o)?;
    let mut r = a;
    for i in 0..25 {
        r[i] += b[i];
    }
    mat_value(vm, r)
}

fn mat_sub(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let a = mat_items(vm, &args[0])?;
    let b = if is_number(&args[1]) { [py::f64_of(vm, &args[1])?; 25] } else { mat_items(vm, &args[1])? };
    let mut r = a;
    for i in 0..25 {
        r[i] -= b[i];
    }
    mat_value(vm, r)
}

fn mat_rsub(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let a = mat_items(vm, &args[0])?;
    let b = if is_number(&args[1]) { [py::f64_of(vm, &args[1])?; 25] } else { mat_items(vm, &args[1])? };
    let mut r = b;
    for i in 0..25 {
        r[i] -= a[i];
    }
    mat_value(vm, r)
}

fn mat_vector_mul(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let s = mat_items(vm, &args[0])?;
    let o = py::items(vm, &args[1])?;
    let mut v = [0.; 4];
    for (i, x) in o.iter().take(4).enumerate() {
        v[i] = py::f64_of(vm, x)?;
    }
    let row = |r: usize| v[0] * s[r * 5] + v[1] * s[r * 5 + 1] + v[2] * s[r * 5 + 2] + v[3] * s[r * 5 + 3] + s[r * 5 + 4];
    Ok(vm.types.new_tuple(vec![Value::Float(row(0)), Value::Float(row(1)), Value::Float(row(2)), Value::Float(row(3)), Value::Int(1)]))
}

fn mat_scalar_mul(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let a = mat_items(vm, &args[0])?;
    let f = py::f64_of(vm, &args[1])?;
    mat_value(vm, a.map(|v| v * f))
}

fn mat_repr(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let a = mat_items(vm, &args[0])?;
    let rows: Vec<String> = (0..5).map(|r| (0..5).map(|c| format!("{:.6}", a[r * 5 + c])).collect::<Vec<_>>().join(", ")).collect();
    Ok(vm.types.new_string(format!("im.matrix({})", rows.join(",\n          "))))
}

fn m20(vm: &mut Vm, v: [f64; 20]) -> PyResult<Value> {
    let mut m = [0.; 25];
    m[..20].copy_from_slice(&v);
    m[24] = 1.;
    mat_value(vm, m)
}

fn sm_identity(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    m20(vm, [1., 0., 0., 0., 0., 0., 1., 0., 0., 0., 0., 0., 1., 0., 0., 0., 0., 0., 1., 0.])
}

fn desat_arg(vm: &mut Vm, args: &[Value], kw: Kw, i: usize) -> PyResult<[f64; 3]> {
    match arg(args, kw, i, "desat") {
        Some(d) => {
            let d = py::items(vm, d)?;
            Ok([py::f64_of(vm, &d[0])?, py::f64_of(vm, &d[1])?, py::f64_of(vm, &d[2])?])
        }
        None => Ok([0.2126, 0.7152, 0.0722]),
    }
}

fn sm_saturation(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let level = py::f64_of(vm, &arg_or(args, kw, 0, "level", Value::Float(1.)))?;
    let [r, g, b] = desat_arg(vm, args, kw, 1)?;
    let i = |a: f64, b: f64| a + (b - a) * level;
    m20(vm, [i(r, 1.), i(g, 0.), i(b, 0.), 0., 0., i(r, 0.), i(g, 1.), i(b, 0.), 0., 0., i(r, 0.), i(g, 0.), i(b, 1.), 0., 0., 0., 0., 0., 1., 0.])
}

fn sm_desaturate(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    sm_saturation(vm, &[Value::Float(0.)], &[])
}

fn sm_tint(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let r = py::f64_of(vm, &args[0])?;
    let g = py::f64_of(vm, &args[1])?;
    let b = py::f64_of(vm, &args[2])?;
    m20(vm, [r, 0., 0., 0., 0., 0., g, 0., 0., 0., 0., 0., b, 0., 0., 0., 0., 0., 1., 0.])
}

fn sm_invert(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    m20(vm, [-1., 0., 0., 0., 1., 0., -1., 0., 0., 1., 0., 0., -1., 0., 1., 0., 0., 0., 1., 0.])
}

fn sm_brightness(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let b = py::f64_of(vm, &args[0])?;
    m20(vm, [1., 0., 0., 0., b, 0., 1., 0., 0., b, 0., 0., 1., 0., b, 0., 0., 0., 1., 0.])
}

fn sm_opacity(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let o = py::f64_of(vm, &args[0])?;
    m20(vm, [1., 0., 0., 0., 0., 0., 1., 0., 0., 0., 0., 0., 1., 0., 0., 0., 0., 0., o, 0.])
}

fn raw20(v: [f64; 20]) -> [f64; 25] {
    let mut m = [0.; 25];
    m[..20].copy_from_slice(&v);
    m[24] = 1.;
    m
}

fn sm_contrast(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let c = py::f64_of(vm, &args[0])?;
    let b = |x: f64| raw20([1., 0., 0., 0., x, 0., 1., 0., 0., x, 0., 0., 1., 0., x, 0., 0., 0., 1., 0.]);
    let t = raw20([c, 0., 0., 0., 0., 0., c, 0., 0., 0., 0., 0., c, 0., 0., 0., 0., 0., 1., 0.]);
    mat_value(vm, mat_mul_raw(&mat_mul_raw(&b(-0.5), &t), &b(0.5)))
}

fn sm_hue(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let h = py::f64_of(vm, &args[0])?.to_radians();
    let (s, c) = h.sin_cos();
    let (lr, lg, lb) = (0.213, 0.715, 0.072);
    let mut m = [0.; 25];
    let v = [
        lr + c * (1. - lr) + s * -lr, lg + c * -lg + s * -lg, lb + c * -lb + s * (1. - lb), 0., 0.,
        lr + c * -lr + s * 0.143, lg + c * (1. - lg) + s * 0.14, lb + c * -lb + s * -0.283, 0., 0.,
        lr + c * -lr + s * -(1. - lr), lg + c * -lg + s * lg, lb + c * (1. - lb) + s * lb, 0., 0.,
        0., 0., 0., 1., 0., 0., 0., 0., 0., 1.,
    ];
    m.copy_from_slice(&v);
    mat_value(vm, m)
}

fn sm_colorize(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let b = super::color::rgba(vm, &args[0])?.map(|v| v as f64 / 255.);
    let w = super::color::rgba(vm, &args[1])?.map(|v| v as f64 / 255.);
    m20(vm, [w[0] - b[0], 0., 0., 0., b[0], 0., w[1] - b[1], 0., 0., b[1], 0., 0., w[2] - b[2], 0., b[2], 0., 0., 0., 1., 0.])
}

// renpy.display.im.Cache: the script-side face of the image cache. Images
// decode and upload on the host (ImageMetadata jobs published as layer
// resources), so the cache holds job states, not surfaces; GPU memory is
// the host's to bound, and no preload thread exists.

/// Pixels held by processed images (upstream counts surface sizes).
fn cache_pixels() -> i64 {
    cache().borrow().entries.values().map(|(_, [w, h])| *w as i64 * *h as i64).sum()
}

fn cache_get_size(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Int(cache_pixels()))
}

fn c_total_size(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Int(cache_pixels()))
}

fn c_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    py::set(vm, this, "time", Value::Int(0))?;
    py::set(vm, this, "cache_limit", Value::Int(0))?;
    py::set(vm, this, "preloads", vm.types.new_list(Vec::new()))?;
    py::set(vm, this, "first_preload_in_tick", Value::Bool(true))?;
    py::set(vm, this, "keep_preloading", Value::Bool(true))?;
    py::set(vm, this, "preload_blacklist", vm.types.new_set(Default::default()))?;
    py::set(vm, this, "preload_thread", Value::None)?;
    py::set(vm, this, "added", vm.types.new_set(Default::default()))?;
    py::set(vm, this, "load_log", vm.types.new_list(Vec::new()))?;
    Ok(Value::None)
}

fn c_done(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(!pending(vm)))
}

/// `init()`: the cache limit from `config.image_cache_size[_mb]`.
fn c_cache_init(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let ics = py::config(vm, "image_cache_size")?;
    let limit = if !py::is_none(&ics) {
        let w = py::config_f64(vm, "screen_width")?;
        let h = py::config_f64(vm, "screen_height")?;
        (2. * py::f64_of(vm, &ics)? * w * h) as i64
    } else {
        let mb = py::config(vm, "image_cache_size_mb").unwrap_or(Value::Int(300));
        (py::f64_of(vm, &mb)? * 1024. * 1024. / 4.) as i64
    };
    py::set(vm, &args[0], "cache_limit", Value::Int(limit))?;
    Ok(Value::None)
}

fn c_clear(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    clear();
    py::set(vm, &args[0], "preloads", vm.types.new_list(Vec::new()))?;
    py::set(vm, &args[0], "first_preload_in_tick", Value::Bool(true))?;
    let added = py::get(vm, &args[0], "added")?;
    py::method(vm, &added, "clear", Vec::new())?;
    Ok(Value::None)
}

fn c_get_renders(vm: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(vm.types.new_list(Vec::new()))
}

fn c_tick(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let this = &args[0];
    let t = py::get(vm, this, "time")?;
    let t = py::int(vm, &t)?;
    py::set(vm, this, "time", Value::Int(t + 1))?;
    py::set(vm, this, "preloads", vm.types.new_list(Vec::new()))?;
    py::set(vm, this, "first_preload_in_tick", Value::Bool(true))?;
    let added = py::get(vm, this, "added")?;
    py::method(vm, &added, "clear", Vec::new())?;
    if py::config_bool(vm, "debug_image_cache")?
        && let Ok(l) = py::import_attr(vm, "renpy.display", "ic_log")
    {
        let s = py::s(vm, "----");
        py::method(vm, &l, "write", vec![s])?;
    }
    Ok(Value::None)
}

fn c_end_tick(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    py::set(vm, &args[0], "preloads", vm.types.new_list(Vec::new()))?;
    Ok(Value::None)
}

fn c_noop(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::None)
}

fn c_cleanout(_: &mut Vm, _: &[Value], _: Kw) -> PyResult<Value> {
    Ok(Value::Bool(true))
}

fn image_base(vm: &mut Vm, v: &Value) -> PyResult<bool> {
    let ib = py::import_attr(vm, MODULE, "ImageBase")?;
    Ok(ib.obj().is_some_and(|c| ops::isinstance(vm, v, c)))
}

/// `get(image, predict=False, texture=False, render=False)`. Textures are
/// host resources, so texture and render requests return the image's
/// render; prediction starts processing and returns None.
fn c_get(vm: &mut Vm, args: &[Value], kw: Kw) -> PyResult<Value> {
    let image = py::arg_or(args, kw, 1, "image", Value::None);
    if !image_base(vm, &image)? {
        let r = ops::repr(vm, &image)?;
        return Err(py::error(vm, format!("Expected an image of some sort, but got{r}.")));
    }
    let flag = |vm: &mut Vm, i: usize, n: &str| -> PyResult<bool> {
        match py::arg(args, kw, i, n) {
            Some(v) => py::truthy(vm, v),
            None => Ok(false),
        }
    };
    let predict = flag(vm, 2, "predict")?;
    let texture = flag(vm, 3, "texture")? || flag(vm, 4, "render")?;
    if predict {
        let g = graph(vm, &image)?;
        let key = identity_key(vm, &image)?;
        texture_for(vm, &key, &g)?;
        return Ok(Value::None);
    }
    if !texture {
        return Err(py::error(vm, "Image surfaces are not available in this runtime; images are drawn by the host."));
    }
    ib_render_inner(vm, &image)
}

fn c_preload_texture(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    c_get(vm, &[args[0].clone(), args[1].clone()], &[("predict".into(), Value::Bool(true)), ("texture".into(), Value::Bool(true))])
}

fn c_get_texture(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    c_get(vm, &[args[0].clone(), args[1].clone()], &[("texture".into(), Value::Bool(true))])?;
    Ok(Value::None)
}

/// `preload_image(im)`: images predicted for the coming interactions start
/// processing at once (there is no preload thread to queue them for).
fn c_preload_image(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let im = &args[1];
    if !image_base(vm, im)? {
        return Ok(Value::None);
    }
    let added = py::get(vm, &args[0], "added")?;
    if ops::contains(vm, &added, im)? {
        return Ok(Value::None);
    }
    ops::set_add(vm, &added, im.clone())?;
    let blacklist = py::get(vm, &args[0], "preload_blacklist")?;
    if ops::contains(vm, &blacklist, im)? {
        return Ok(Value::None);
    }
    if c_preload_texture(vm, args, &[]).is_err() {
        ops::set_add(vm, &blacklist, im.clone())?;
    }
    Ok(Value::None)
}

/// `flush_file(fn)`: forgets processed images that read `fn`.
fn c_flush_file(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    let f = py::string(vm, &args[1])?;
    let quoted = format!("{f:?}");
    let single = format!("'{f}'");
    let c = cache();
    let before = c.borrow().entries.len();
    c.borrow_mut().entries.retain(|k, _| !(k.contains(&quoted) || k.contains(&single)));
    if c.borrow().entries.len() != before
        && let Ok(fm) = py::import_attr(vm, "renpy.display.render", "free_memory")
    {
        vm.call(&fm, Vec::new(), Vec::new())?;
    }
    Ok(Value::None)
}

fn c_add_load_log(vm: &mut Vm, args: &[Value], _: Kw) -> PyResult<Value> {
    if !py::config_bool(vm, "developer")? {
        return Ok(Value::None);
    }
    let log = py::get(vm, &args[0], "load_log")?;
    let t = py::f(super::link::now(vm));
    let e = vm.types.new_tuple(vec![t, args[1].clone(), Value::Bool(false)]);
    py::method(vm, &log, "insert", vec![Value::Int(0), e])?;
    while ops::len(vm, &log)? > 100 {
        py::method(vm, &log, "pop", Vec::new())?;
    }
    Ok(Value::None)
}

pub fn init_module(vm: &mut Vm) -> PyResult<ObjRef> {
    let m = crysirror_python::modules::module_with(vm, MODULE, &[
        ("image", f_image),
        ("ramp", f_ramp),
        ("Grayscale", f_grayscale),
        ("Sepia", f_sepia),
        ("Color", f_color),
        ("Alpha", f_alpha),
        ("load_image", f_load_image),
        ("load_surface", f_load_image),
        ("load_rgba", f_load_rgba),
        ("unoptimized_texture", f_unoptimized_texture),
        ("render_for_texture", f_render_for_texture),
        ("free_memory", |_, _, _| {
            clear();
            Ok(Value::None)
        }),
        ("reset_module", |_, _, _| {
            clear();
            Ok(Value::None)
        }),
    ]);
    let set = |vm: &mut Vm, n: &str, v: Value| crysirror_python::modules::set(vm, &m, n, v);
    let d = Value::Obj(super::core::displayable_class(vm));
    let c = |vm: &mut Vm, name: &str, bases: &[Value], methods: Methods, attrs: &[(&str, Value)]| -> PyResult<Value> {
        let cls = py::class(vm, MODULE, name, bases, methods, attrs)?;
        let v = Value::Obj(cls);
        crysirror_python::modules::set(vm, &m, name, v.clone());
        Ok(v)
    };
    let empty = vm.types.new_list(Vec::new());
    let base = c(vm, "ImageBase", &[d], &[
        ("__init__", ib_init),
        ("after_upgrade", ib_after_upgrade),
        ("__hash__", ib_hash),
        ("__eq__", ib_eq),
        ("load", ib_load),
        ("render", ib_render),
        ("get_placement", ib_get_placement),
        ("predict_one", ib_predict_one),
        ("predict_files", ib_predict_files),
        ("get_hash", ib_zero),
        ("get_oversample", ib_get_oversample),
    ], &[
        ("__version__", Value::Int(1)), ("optimize_bounds", Value::Bool(false)), ("oversample", Value::Int(1)),
        ("pixel_perfect", Value::Bool(false)), ("obsolete", Value::Bool(true)), ("obsolete_list", empty), ("fail", Value::None),
    ])?;
    c(vm, "Image", &[base.clone()], &[("__init__", image_init), ("_repr_info", image_repr_info), ("get_hash", image_get_hash)], &[("obsolete", Value::Bool(false)), ("is_svg", Value::Bool(false)), ("dpi", Value::Int(96))])?;
    c(vm, "Data", &[base.clone()], &[("__init__", data_init), ("_repr_info", image_repr_info)], &[("obsolete", Value::Bool(false))])?;
    c(vm, "ZipFileImage", &[base.clone()], &[("__init__", zip_init)], &[("obsolete", Value::Bool(false))])?;
    c(vm, "Composite", &[base.clone()], &[("__init__", composite_init), ("get_hash", composite_get_hash)], &[])?;
    for (name, init) in [
        ("Scale", scale_init as crysirror_python::object::NativeFn),
        ("FactorScale", factorscale_init),
        ("Flip", flip_init),
        ("Rotozoom", rotozoom_init),
        ("Crop", crop_init),
        ("Map", map_init),
        ("Twocolor", twocolor_init),
        ("Recolor", recolor_init),
        ("Blur", blur_init),
        ("MatrixColor", matrixcolor_init),
        ("Tile", tile_init),
    ] {
        c(vm, name, &[base.clone()], &[("__init__", init), ("get_hash", ib_child_hash)], &[])?;
    }
    c(vm, "AlphaMask", &[base.clone()], &[("__init__", alphamask_init), ("get_hash", alphamask_get_hash)], &[])?;
    c(vm, "UnoptimizedTexture", &[base], &[("__init__", unopt_init), ("get_hash", ib_child_hash)], &[("obsolete", Value::Bool(false))])?;
    let id = identity_ramp(vm);
    set(vm, "identity", id);
    set(vm, "ramp_cache", vm.types.new_dict());
    let empty_set = vm.call(&Value::Obj(vm.types.set.clone()), Vec::new(), Vec::new())?;
    set(vm, "ignored_images", empty_set.clone());
    set(vm, "images_to_ignore", empty_set);
    // im.matrix
    let tuple = Value::Obj(vm.types.tuple.clone());
    let mc = c(vm, "matrix", &[tuple], &[
        ("mul", |vm, a, k| mat_mul(vm, &[a[1].clone(), a[2].clone()], k)),
        ("scalar_mul", mat_scalar_mul),
        ("vector_mul", mat_vector_mul),
        ("__add__", mat_add),
        ("__radd__", mat_add),
        ("__sub__", mat_sub),
        ("__rsub__", mat_rsub),
        ("__mul__", mat_mul),
        ("__rmul__", mat_rmul),
        ("__repr__", mat_repr),
    ], &[])?;
    let mco = mc.obj().cloned().unwrap();
    let n = vm.types.new_builtin("__new__", mat_new);
    let n = py::staticmethod(vm, n);
    vm.types.set_attr(&mco, "__new__", n);
    for (name, f) in [
        ("identity", sm_identity as crysirror_python::object::NativeFn),
        ("saturation", sm_saturation),
        ("desaturate", sm_desaturate),
        ("tint", sm_tint),
        ("invert", sm_invert),
        ("brightness", sm_brightness),
        ("opacity", sm_opacity),
        ("contrast", sm_contrast),
        ("hue", sm_hue),
        ("colorize", sm_colorize),
    ] {
        let b = vm.types.new_builtin(name, f);
        let s = py::staticmethod(vm, b);
        vm.types.set_attr(&mco, name, s);
    }
    let cache_cls = py::class(vm, MODULE, "Cache", &[], &[
        ("__init__", c_init),
        ("done", c_done),
        ("get_total_size", c_total_size),
        ("get_current_size", cache_get_size),
        ("init", c_cache_init),
        ("quit", c_clear),
        ("clear", c_clear),
        ("get_renders", c_get_renders),
        ("tick", c_tick),
        ("end_tick", c_end_tick),
        ("get", c_get),
        ("kill", c_noop),
        ("cleanout", c_cleanout),
        ("flush_file", c_flush_file),
        ("preload_texture", c_preload_texture),
        ("get_texture", c_get_texture),
        ("preload_image", c_preload_image),
        ("start_prediction", c_noop),
        ("preload_thread_main", c_noop),
        ("preload_thread_pass", c_noop),
        ("add_load_log", c_add_load_log),
    ], &[])?;
    let cache_obj = vm.call(&Value::Obj(cache_cls.clone()), Vec::new(), Vec::new())?;
    set(vm, "Cache", Value::Obj(cache_cls));
    set(vm, "cache", cache_obj);
    Ok(m)
}

pub fn install(vm: &mut Vm) {
    vm.register_native_module(MODULE, init_module);
}
