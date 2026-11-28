// SPDX-License-Identifier: MPL-2.0
//! Ren'Py adopt adapter: script loading, AST execution and the renpy
//! runtime over shared CrySirror services.
#![forbid(unsafe_code)]

pub mod adapter;
pub mod ast;
pub mod atl;
mod atl_runtime;
mod audio;
mod display;
pub mod engine;
mod exec;
pub mod files;
pub(crate) mod glue;
mod host_audio;
pub mod lexer;
mod natives;
pub mod parser;
mod persistent;
mod pycompile;
mod rollback;
pub mod rpyc;
pub mod rt;
mod scene;
mod screen_runtime;
mod screens;
pub mod script;
pub mod sl2;
mod statements;
mod style;
pub mod translation;
