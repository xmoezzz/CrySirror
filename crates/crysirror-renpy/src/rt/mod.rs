// SPDX-License-Identifier: MPL-2.0
//! The native Ren'Py runtime: Rust implementations of the upstream
//! display, text, audio and engine modules, exposed under their upstream
//! module names. Script-visible classes are ordinary Python classes whose
//! methods are natives; the small Python shells in `py/renpy` only declare
//! module layout.
pub mod anim;
pub mod astexec;
pub mod atlrt;
pub mod audio;
pub mod backend;
pub mod behavior;
pub mod behavior2;
pub mod boot;
pub mod character;
pub mod color;
pub mod core;
pub mod dragdrop;
pub mod easy;
pub mod error;
pub mod events;
pub mod execution;
pub mod exports;
pub mod focus;
pub mod font;
pub mod game;
pub mod im;
pub mod image;
pub mod imagelike;
pub mod imagemap;
pub mod input;
pub mod interface;
pub mod layout;
pub mod lexer;
pub mod link;
pub mod loader;
pub mod loadsave;
pub mod log;
pub mod main;
pub mod media;
pub mod motion;
pub mod movetransition;
pub mod nodes;
pub mod parameter;
pub mod params;
pub mod parser;
pub mod particle;
pub mod persistent;
pub mod predict;
pub mod preference_tables;
pub mod py;
pub mod python;
pub mod render;
pub mod resloader;
pub mod revertable;
pub mod rollback;
pub mod savestore;
pub mod savetoken;
pub mod scenelists;
pub mod screen;
pub mod scriptedit;
pub mod slast;
pub mod sldisplayables;
pub mod slparser;
pub mod slproperty_names;
pub mod state;
pub mod statements;
pub mod style;
pub mod substitutions;
pub mod support;
pub mod testing;
pub mod text;
pub mod textshader;
pub mod transform;
pub mod transition;
pub mod translation;
pub mod ui;
pub mod viewport;

use crysirror_core::service::{FontLibrary, ImageMetadata};
use crysirror_python::{PyResult, Vm};
use std::{cell::RefCell, rc::Rc, sync::Arc};

/// Host-side services available to natives during script execution. The
/// adapter fills these at prepare time; host calls are synchronous queries
/// on shared, client-scoped services.
pub struct Host {
    pub images: Option<Arc<dyn ImageMetadata>>,
    pub fonts: Option<Arc<dyn FontLibrary>>,
    /// Game-relative file name -> layer asset path.
    pub assets: Option<Rc<dyn Fn(&str) -> Option<String>>>,
    pub title: Option<String>,
    pub clipboard: String,
    pub canvas: [i32; 2],
    /// The client's Save mount; saves stay in memory without one.
    pub saves: Option<Arc<dyn crysirror_core::service::ClientStorage>>,
    pub save_store: Option<Rc<savestore::SaveStore>>,
    /// Screen variants of the host, most specific first (upstream's
    /// desktop default is `large`, `pc`).
    pub variants: Vec<String>,
}

impl Default for Host {
    fn default() -> Host {
        Host {
            images: None,
            fonts: None,
            assets: None,
            title: None,
            clipboard: String::new(),
            canvas: [0, 0],
            saves: None,
            save_store: None,
            variants: vec!["large".into(), "pc".into()],
        }
    }
}

pub fn host(vm: &Vm) -> Rc<RefCell<Host>> {
    vm.ext_get::<Rc<RefCell<Host>>>().expect("Ren'Py host services").clone()
}

pub fn clipboard_text(vm: &Vm) -> String {
    host(vm).borrow_mut().clipboard.clone()
}

pub fn set_clipboard_text(vm: &Vm, text: String) {
    host(vm).borrow_mut().clipboard = text;
}

pub fn set_window_title(vm: &Vm, text: String) {
    host(vm).borrow_mut().title = Some(text);
}

pub fn asset_path(vm: &Vm, name: &str) -> Option<String> {
    let f = host(vm).borrow_mut().assets.clone()?;
    f(name)
}

/// Registers every native module. Called once per VM before `import renpy`.
pub fn install(vm: &mut Vm) -> PyResult<()> {
    vm.ext_set(Rc::new(RefCell::new(Host::default())));
    render::install(vm);
    focus::install(vm);
    style::install(vm);
    events::install(vm);
    image::install(vm);
    image::install_module(vm);
    core::install(vm);
    layout::install(vm);
    color::install(vm);
    easy::install(vm);
    im::install(vm);
    imagelike::install(vm);
    font::install(vm);
    text::install(vm);
    transform::install(vm);
    atlrt::install(vm);
    motion::install(vm);
    transition::install(vm);
    movetransition::install(vm);
    behavior::install(vm);
    anim::install(vm);
    viewport::install(vm);
    dragdrop::install(vm);
    particle::install(vm);
    imagemap::install(vm);
    ui::install(vm);
    predict::install(vm);
    slast::install(vm);
    screen::install(vm);
    scenelists::install(vm);
    interface::install(vm);
    state::install_host_module(vm);
    game::install(vm);
    translation::install(vm);
    exports::install(vm);
    revertable::install(vm);
    python::install(vm);
    rollback::install(vm);
    execution::install(vm);
    astexec::install(vm);
    nodes::install(vm);
    sldisplayables::install(vm);
    lexer::install(vm);
    parameter::install(vm);
    parser::install(vm);
    statements::install(vm);
    support::install(vm);
    character::install(vm);
    substitutions::install(vm);
    resloader::install(vm);
    savetoken::install(vm);
    persistent::install(vm);
    loadsave::install(vm);
    audio::install(vm);
    media::install(vm);
    slparser::install(vm);
    textshader::install(vm);
    log::install(vm);
    error::install(vm);
    main::install(vm);
    scriptedit::install(vm);
    testing::install(vm);
    input::install(vm);
    Ok(())
}

