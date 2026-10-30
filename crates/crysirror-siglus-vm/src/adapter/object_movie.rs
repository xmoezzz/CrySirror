// Copyright (c) 2026 CrySirror contributors.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. A copy is available at https://mozilla.org/MPL/2.0/.
//! Original OBJECT movie lifetime/locator policy on shared engine Media.
//! The engine owns metadata admission, decoding, audio, clocks and pictures.
use super::*;
use crate::runtime::globals::ObjectState;
use crysirror_core::service::{MediaPhase, MediaRequest};
pub(crate) type Path = Vec<usize>;
#[derive(Clone)]
pub(crate) struct Picture {
    pub surface: String,
    pub size: [u32; 2],
    pub ready: bool,
}
pub(crate) type Pictures = BTreeMap<Path, Picture>;
struct Binding {
    id: u32,
    revision: u64,
    file: String,
    looping: bool,
    paused: bool,
    real: bool,
    surface: String,
    ended: bool,
}
#[derive(Default)]
pub(super) struct ObjectMovies {
    bindings: BTreeMap<Path, Binding>,
    pub pictures: Pictures,
}
#[derive(Clone)]
struct Declaration {
    path: Path,
    file: String,
    revision: u64,
    looping: bool,
    paused: bool,
    real: bool,
    sample_ms: u64,
}
fn object_mut<'a>(ctx: &'a mut CommandContext, path: &[usize]) -> Option<&'a mut ObjectState> {
    let mut object = ctx
        .globals
        .stage_forms
        .values_mut()
        .next()?
        .object_lists
        .get_mut(&1)?
        .get_mut(*path.first()?)?;
    for index in &path[1..] {
        object = object.runtime.child_objects.get_mut(*index)?;
    }
    Some(object)
}
fn declarations(ctx: &CommandContext) -> Result<Vec<Declaration>> {
    ensure!(
        ctx.globals.stage_forms.len() <= 1,
        "object Media supports one native stage namespace"
    );
    let mut todo = vec![];
    let mut out = vec![];
    for stage in ctx.globals.stage_forms.values() {
        for (stage_idx, objects) in &stage.object_lists {
            for (index, o) in objects.iter().enumerate() {
                todo.push((*stage_idx, vec![index], o));
            }
        }
        // Other native locations need their own stable locator/save policy;
        // never silently hide a video in a message-window subtree.
        for w in stage.mwnd_lists.values().flatten() {
            let mut nested = w
                .object_list
                .iter()
                .chain(&w.face_list)
                .chain(&w.button_list)
                .collect::<Vec<_>>();
            while let Some(o) = nested.pop() {
                ensure!(
                    o.object_type != 9,
                    "native MWND object movie location is not mapped"
                );
                nested.extend(&o.runtime.child_objects);
            }
        }
    }
    let mut visited = 0;
    while let Some((stage_idx, path, object)) = todo.pop() {
        visited += 1;
        ensure!(
            visited <= 4096 && path.len() <= 17,
            "native object Media tree budget"
        );
        if object.object_type == 9 {
            ensure!(
                stage_idx == 1,
                "native object Media non-FRONT stage location is not mapped"
            );
            ensure!(
                !ctx.excall_state.ready,
                "native EXCALL object Media clock scope is not mapped"
            );
            let sample_ms = u64::try_from(if object.movie.real_time_flag {
                ctx.globals.local_real_time
            } else {
                ctx.globals.local_game_time
            })
            .context("native object Media clock must be nonnegative")?;
            out.push(Declaration {
                path: path.clone(),
                file: object
                    .file_name
                    .clone()
                    .context("native object movie filename")?,
                revision: object.movie.engine_revision,
                looping: object.movie.loop_flag,
                // These declarations belong to the ordinary local stage. The
                // native VM decides whether that stage receives time; shared
                // Media owns the resulting pause offsets and playback clock.
                paused: object.movie.pause_flag
                    || ctx.globals.script.time_stop_flag
                    || ctx.globals.script.stage_time_stop_flag,
                real: object.movie.real_time_flag,
                sample_ms,
            });
            ensure!(out.len() <= 16, "native object Media count budget");
        }
        for (index, child) in object.runtime.child_objects.iter().enumerate() {
            let mut child_path = path.clone();
            child_path.push(index);
            todo.push((stage_idx, child_path, child));
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}
pub(super) fn has_movies(ctx: &CommandContext) -> bool {
    let mut todo = ctx
        .globals
        .stage_forms
        .values()
        .flat_map(|s| {
            s.object_lists
                .values()
                .flatten()
                .chain(s.mwnd_lists.values().flatten().flat_map(|w| {
                    w.object_list
                        .iter()
                        .chain(&w.face_list)
                        .chain(&w.button_list)
                }))
        })
        .collect::<Vec<_>>();
    while let Some(o) = todo.pop() {
        if o.object_type == 9 {
            return true;
        }
        todo.extend(&o.runtime.child_objects);
    }
    false
}
impl ObjectMovies {
    pub(super) fn active(&self) -> bool {
        !self.bindings.is_empty()
    }
}
impl SiglusVm {
    pub(super) fn sync_object_movies(&mut self, host: &mut dyn HostServices) -> Result<bool> {
        let Some(vm) = self.state.vm.as_ref() else {
            return Ok(true);
        };
        let wanted = declarations(&vm.ctx)?;
        let retire = self
            .object_movies
            .bindings
            .iter()
            .filter(|(path, binding)| {
                !wanted.iter().any(|d| {
                    &d.path == *path && d.revision == binding.revision && d.file == binding.file
                })
            })
            .map(|(path, _)| path.clone())
            .collect::<Vec<_>>();
        for path in retire {
            let binding = &self.object_movies.bindings[&path];
            host.media_request(
                ClientId(self.state.client),
                MediaRequest::Remove { id: binding.id },
            )?;
            self.object_movies.bindings.remove(&path);
            self.object_movies.pictures.remove(&path);
        }
        let mut ready = true;
        for declaration in wanted {
            if !self.object_movies.bindings.contains_key(&declaration.path) {
                let asset = self.movie_asset(host, &declaration.file)?;
                // Shared Media identities are client-local 0..1024. Reserve
                // 1023 for native GLOBAL.MOV and reuse only fully removed slots.
                let id = (0..super::movie::MEDIA_ID)
                    .find(|id| !self.object_movies.bindings.values().any(|b| b.id == *id))
                    .context("native object Media identities exhausted")?;
                let gain = self.native_movie_gain();
                let surface = host
                    .media_request(
                        ClientId(self.state.client),
                        MediaRequest::Play {
                            id,
                            asset,
                            looping: declaration.looping,
                            audio: true,
                            gain,
                        },
                    )?
                    .relative()
                    .to_owned();
                host.media_request(
                    ClientId(self.state.client),
                    MediaRequest::SetClock {
                        id,
                        now_ms: declaration.sample_ms,
                    },
                )?;
                if declaration.paused {
                    host.media_request(ClientId(self.state.client), MediaRequest::Pause { id })?;
                }
                self.object_movies.bindings.insert(
                    declaration.path.clone(),
                    Binding {
                        id,
                        revision: declaration.revision,
                        file: declaration.file.clone(),
                        looping: declaration.looping,
                        paused: declaration.paused,
                        real: declaration.real,
                        surface,
                        ended: false,
                    },
                );
            }
            let binding = self
                .object_movies
                .bindings
                .get_mut(&declaration.path)
                .unwrap();
            ensure!(
                binding.real == declaration.real,
                "native object Media clock domain mutation requires recreation"
            );
            // Always publish the native source sample before a pause/resume
            // edge. Engine Media compensates for elapsed caller-clock time.
            host.media_request(
                ClientId(self.state.client),
                MediaRequest::SetClock {
                    id: binding.id,
                    now_ms: declaration.sample_ms,
                },
            )?;
            ensure!(
                binding.looping == declaration.looping,
                "native end-loop control requires shared Media loop mutation"
            );
            if binding.paused != declaration.paused {
                host.media_request(
                    ClientId(self.state.client),
                    if declaration.paused {
                        MediaRequest::Pause { id: binding.id }
                    } else {
                        MediaRequest::Resume { id: binding.id }
                    },
                )?;
                binding.paused = declaration.paused;
            }
            let status = host.media_info(ClientId(self.state.client), binding.id)?;
            ensure!(
                status.phase != MediaPhase::Failed,
                "engine object Media failed: {}",
                binding.file
            );
            let frame = if status.published_frames > 0 {
                host.media_frame(ClientId(self.state.client), binding.id)?
            } else {
                None
            };
            let size = status.size.or_else(|| frame.as_ref().map(|f| f.size()));
            ready &= size.is_some();
            let object = object_mut(&mut self.state.vm.as_mut().unwrap().ctx, &declaration.path)
                .context("native object Media target disappeared")?;
            let previous = object.movie.timer_ms;
            object.movie.timer_ms = status.position_ms;
            object.movie.total_ms = status.duration_ms;
            if let Some(size) = size {
                object.movie.width = size[0];
                object.movie.height = size[1];
            }
            let ended = matches!(status.phase, MediaPhase::Ended | MediaPhase::Stopped);
            object.movie.just_finished = ended && !binding.ended;
            object.movie.just_looped = declaration.looping && status.position_ms < previous;
            if ended {
                object.movie.playing = false;
            }
            binding.ended = ended;
            self.object_movies.pictures.insert(
                declaration.path.clone(),
                Picture {
                    surface: binding.surface.clone(),
                    size: size.unwrap_or([1, 1]),
                    ready: frame.is_some(),
                },
            );
            if ended && object.movie.auto_free_flag {
                // Original EOS releases only the type-owned resource; CHILD
                // and render/callback state survive init_type(true).
                object.init_type_like();
                host.media_request(
                    ClientId(self.state.client),
                    MediaRequest::Remove { id: binding.id },
                )?;
                self.object_movies.bindings.remove(&declaration.path);
                self.object_movies.pictures.remove(&declaration.path);
            }
        }
        Ok(ready)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_clock_selection_and_stage_pause_are_semantic_declarations() {
        let mut ctx = CommandContext::new("asset".into());
        let mut object = ObjectState::default();
        object.object_type = 9;
        object.file_name = Some("clip".into());
        object.movie.start(None, false, true, false, false);
        ctx.globals
            .stage_forms
            .entry(38)
            .or_default()
            .object_lists
            .insert(1, vec![object]);
        ctx.globals.local_game_time = 320;
        ctx.globals.local_real_time = 10;
        let d = declarations(&ctx).unwrap().remove(0);
        assert_eq!(d.sample_ms, 320);
        assert!(!d.real && !d.paused);
        ctx.globals.script.stage_time_stop_flag = true;
        assert!(declarations(&ctx).unwrap()[0].paused);
        assert!(
            !ctx.globals.stage_forms[&38].object_lists[&1][0]
                .movie
                .pause_flag,
            "ordinary stage suspension does not rewrite the native object's own pause flag"
        );
        ctx.globals.script.stage_time_stop_flag = false;
        let object = object_mut(&mut ctx, &[0]).unwrap();
        object.movie.real_time_flag = true;
        object.movie.pause_flag = true;
        let d = declarations(&ctx).unwrap().remove(0);
        assert_eq!(d.sample_ms, 10);
        assert!(d.real && d.paused);
        object_mut(&mut ctx, &[0]).unwrap().movie.pause_flag = false;
        ctx.globals.script.time_stop_flag = true;
        assert!(declarations(&ctx).unwrap()[0].paused);
        ctx.excall_state.ready = true;
        assert!(
            declarations(&ctx).is_err(),
            "a separate EXCALL stage clock is not guessed"
        );
    }
}
