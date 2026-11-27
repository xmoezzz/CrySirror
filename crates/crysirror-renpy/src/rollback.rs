// SPDX-License-Identifier: MPL-2.0
//! Interaction checkpoints for Ren'Py's script owner.

use crate::engine::{Context, Engine, Interaction, Wait};
use crysirror_python::{Value, checkpoint::Checkpoint, import, ops};
use std::collections::{BTreeMap, VecDeque};

const HISTORY_BYTES: usize = 64 * 1024 * 1024;
const MAX_CHECKPOINT_BYTES: usize = 8 * 1024 * 1024;

struct Entry {
    checkpoint: Checkpoint,
    contexts: Vec<Context>,
    interaction: Interaction,
    waits: Vec<Wait>,
    cursor: Option<usize>,
    loops: BTreeMap<u32, crate::audio::Command>,
    resumed_with: Option<Decision>,
}

#[derive(Clone)]
struct Decision {
    node: Option<usize>,
    choice: Option<usize>,
    timeout: bool,
    value: Value,
}

#[derive(Default)]
pub(crate) struct History {
    entries: VecDeque<Entry>,
    forward: VecDeque<Decision>,
    bytes: usize,
}

impl Engine {
    fn rollback_limit(&self) -> usize {
        let config = import::get_module(&self.vm, "renpy.config");
        let value = |name| {
            config
                .as_ref()
                .and_then(|module| ops::get_raw_attr(module.obj()?, name))
        };
        if matches!(value("rollback_enabled"), Some(Value::Bool(false)))
            || matches!(
                self.store_dict("store").dict_get_str_raw("_rollback"),
                Some(Value::Bool(false))
            )
        {
            return 0;
        }
        match value("rollback_length") {
            Some(Value::Int(length)) => length.clamp(0, 128) as usize,
            _ => 128,
        }
    }

    pub(crate) fn push_checkpoint(&mut self) {
        let limit = self.rollback_limit();
        if limit == 0 || self.st.rollback_suspended.get() {
            return;
        }
        let Some(interaction) = self.interaction().filter(|i| {
            !matches!(
                i,
                Interaction::Transition { .. } | Interaction::Screen { .. }
            )
        }) else {
            return;
        };
        let contexts = self.st.contexts.borrow();
        if !contexts
            .last()
            .is_some_and(|context| context.rollback && !context.init_phase)
        {
            return;
        }
        let store = self.store_dict("store");
        let values = self.st.store.borrow();
        let roots = std::iter::once(Value::Obj(store.clone()))
            .chain(values.values().cloned().map(Value::Obj))
            .chain(contexts.iter().flat_map(|context| {
                std::iter::once(context.py.clone())
                    .chain(context.screens.iter().flat_map(|screen| {
                        screen
                            .args
                            .iter()
                            .cloned()
                            .chain(screen.kwargs.iter().map(|(_, value)| value.clone()))
                    }))
                    .chain(
                        context
                            .dynamic_stack
                            .iter()
                            .flatten()
                            .filter_map(|(_, value)| value.clone()),
                    )
            }))
            .chain(self.waits.iter().flat_map(|wait| match wait {
                Wait::Interaction { task, .. }
                | Wait::Task { task, .. }
                | Wait::NestedCall { task, .. } => task.retained_values(),
            }));
        let marker = import::get_module(&self.vm, "renpy.object")
            .and_then(|module| ops::get_raw_attr(module.obj()?, "NoRollback"));
        let persistent = store.dict_get_str_raw("persistent");
        let preferences = store.dict_get_str_raw("_preferences");
        let checkpoint = Checkpoint::capture(
            roots,
            |object| {
                persistent.iter().chain(&preferences).any(|value| {
                    value
                        .obj()
                        .is_some_and(|excluded| std::rc::Rc::ptr_eq(object, excluded))
                }) || marker.as_ref().and_then(Value::obj).is_some_and(|marker| {
                    object.class().type_data().is_some_and(|class| {
                        class
                            .mro
                            .borrow()
                            .iter()
                            .any(|base| std::rc::Rc::ptr_eq(base, marker))
                    })
                })
            },
            MAX_CHECKPOINT_BYTES,
        );
        let checkpoint = match checkpoint {
            Ok(checkpoint) => checkpoint,
            Err(reason) => {
                drop(values);
                drop(contexts);
                self.reset_history();
                self.st
                    .log
                    .borrow_mut()
                    .push(format!("Ren'Py rollback history cleared: {reason}"));
                return;
            }
        };
        let bytes = checkpoint.byte_count();
        while self.history.entries.len() >= limit || self.history.bytes + bytes > HISTORY_BYTES {
            let old = self
                .history
                .entries
                .pop_front()
                .expect("bounded checkpoint history");
            self.history.bytes -= old.checkpoint.byte_count();
        }
        self.history.bytes += bytes;
        self.history.entries.push_back(Entry {
            checkpoint,
            contexts: contexts.clone(),
            interaction,
            waits: self.waits.clone(),
            cursor: self.cursor,
            loops: self.st.audio_loops.borrow().clone(),
            resumed_with: None,
        });
        self.st
            .rollback_available
            .set(self.history.entries.len().saturating_sub(1));
        self.update_roll_forward();
    }

    pub fn rewind(&mut self, count: usize) -> bool {
        if self.rollback_limit() == 0 || count == 0 {
            return false;
        }
        let distance = if self.interaction().is_some() {
            count.saturating_add(1)
        } else {
            count
        };
        let Some(index) = self.history.entries.len().checked_sub(distance) else {
            return false;
        };
        let mut forward = VecDeque::new();
        for entry in self.history.entries.iter().skip(index) {
            if let Some(decision) = &entry.resumed_with {
                forward.push_back(decision.clone());
            }
        }
        forward.append(&mut self.history.forward);
        self.history.forward = forward;
        while self.history.entries.len() > index + 1 {
            let entry = self
                .history
                .entries
                .pop_back()
                .expect("rewind destination is present");
            self.history.bytes -= entry.checkpoint.byte_count();
        }
        let checkpoint = self.history.entries.back().expect("rewind checkpoint");
        checkpoint.checkpoint.restore();
        self.st.contexts.replace(checkpoint.contexts.clone());
        self.st
            .interaction
            .replace(Some(checkpoint.interaction.clone()));
        self.waits.clone_from(&checkpoint.waits);
        self.cursor = checkpoint.cursor;
        self.halted = false;
        self.st.audio_reset.replace(Some(checkpoint.loops.clone()));
        self.st.audio_loops.replace(checkpoint.loops.clone());
        self.st.audio_commands.borrow_mut().clear();
        self.st.rollback_available.set(index);
        self.st.rollback_suspended.set(false);
        self.st.in_rollback.set(true);
        self.update_roll_forward();
        true
    }

    pub(crate) fn reset_history(&mut self) {
        self.history = History::default();
        self.st.rollback_available.set(0);
        self.st.in_rollback.set(false);
        self.st.roll_forward.replace(Value::None);
    }

    fn update_roll_forward(&self) {
        let value = self
            .history
            .forward
            .front()
            .filter(|entry| entry.node == self.cursor)
            .map_or(Value::None, |decision| decision.value.clone());
        self.st.roll_forward.replace(value);
    }

    pub fn roll_forward(&mut self) -> Result<bool, String> {
        let Some(decision) = self
            .history
            .forward
            .front()
            .filter(|entry| entry.node == self.cursor)
            .cloned()
        else {
            return Ok(false);
        };
        self.finish_interaction(decision.choice, decision.timeout, true)?;
        Ok(true)
    }

    pub(crate) fn remember_decision(
        &mut self,
        choice: Option<usize>,
        timeout: bool,
        value: Value,
        replay: bool,
    ) {
        if let Some(entry) = self
            .history
            .entries
            .back_mut()
            .filter(|entry| entry.cursor == self.cursor)
        {
            entry.resumed_with = Some(Decision {
                node: self.cursor,
                choice,
                timeout,
                value,
            });
        }
        if replay {
            self.history.forward.pop_front();
        } else {
            self.history.forward.clear();
        }
        self.st.in_rollback.set(!self.history.forward.is_empty());
        self.update_roll_forward();
    }
}
