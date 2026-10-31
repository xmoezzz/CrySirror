// SPDX-License-Identifier: MPL-2.0
//! Original627E50 task state. This is native resource/scheduling policy, not a
//! hosted FES renderer. The player and its operations remain separately gated.
use anyhow::{ensure, Result};

#[derive(Clone, Debug)]
pub(crate) struct FesTask {
    pub id: u32,
    pub parent: u32,
    pub name: Vec<u8>,
    pub parameters: Vec<u8>,
    /// Parent177 returns the original raw-string bank, not the interpreter.
    pub string_bank_owner: Option<u32>,
    pub interpreter_owner: Option<u32>,
    pub player_loaded: bool,
    pub running: bool,
    pub paused: bool,
    pub first_tick: bool,
    pub low_button: Option<u32>,
    pub image_list: Option<u32>,
    pub native_rect: [i32; 4],
    pub upload_position: Option<[i32; 2]>,
    /// Constructor125 cannot reach INPUT before attach. Output remains unknown;
    /// do not invent a pointer location or INPUT task reference.
    pub saved_cursor: Option<[i32; 2]>,
    pub input_task: Option<u32>,
    pub sound_task: Option<u32>,
    pub child_tasks: [Option<u32>; 2],
    pub exit_code: i32,
    pub selected_object: i32,
    pub click_state: u32,
    pub message_parameter: u32,
    pub script_name: Vec<u8>,
    pub next_object: [u8; 36],
    pub previous_object: [u8; 36],
    pub resource_directory: Vec<u16>,
    pub auxiliary_records: Vec<u32>,
    pub preload_names: Vec<Vec<u8>>,
}
impl FesTask {
    pub fn new(id: u32, parent: u32, name: Vec<u8>, parameters: Vec<u8>) -> Result<Self> {
        ensure!(
            parameters.len() < 32 && !parameters.contains(&0),
            "CatSystem2 native FES parameter extent"
        );
        Ok(Self {
            id,
            parent,
            name,
            parameters,
            string_bank_owner: Some(parent),
            interpreter_owner: None,
            player_loaded: false,
            running: false,
            paused: false,
            first_tick: true,
            low_button: None,
            image_list: None,
            native_rect: [0; 4],
            //627E50 explicitly zeroes1108/1112, unlike image-list112/116.
            upload_position: Some([0, 0]),
            saved_cursor: None,
            input_task: None,
            sound_task: None,
            child_tasks: [None, None],
            exit_code: -1,
            selected_object: -1,
            click_state: 0,
            //Native66 sends26 after attach;62AD10 stores this[298]=lparam0.
            message_parameter: 0,
            script_name: Vec::new(),
            next_object: [0; 36],
            previous_object: [0; 36],
            //Root5DD3C0's UTF16 constant8CD268 is exactly".". The scoped
            //asset root supplies its host location; no native pointer is kept.
            resource_directory: vec![u16::from(b'.')],
            auxiliary_records: Vec::new(),
            preload_names: Vec::new(),
        })
    }
    pub fn diagnostics(&self) -> serde_json::Value {
        serde_json::json!({"id":self.id,"task_parent":self.parent,"name":String::from_utf8_lossy(&self.name),"parameters":self.parameters,"string_bank_owner":self.string_bank_owner,"interpreter_owner":self.interpreter_owner,"player_loaded":self.player_loaded,"running":self.running,"paused":self.paused,"first_tick":self.first_tick,"low_button":self.low_button,"image_list":self.image_list,"native_rect":self.native_rect,"upload_position":self.upload_position,"saved_cursor":self.saved_cursor,"input_task":self.input_task,"sound_task":self.sound_task,"child_tasks":self.child_tasks,"exit_code":self.exit_code,"selected_object":self.selected_object,"click_state":self.click_state,"message_parameter":self.message_parameter,"script_name":self.script_name,"next_object":self.next_object.as_slice(),"previous_object":self.previous_object.as_slice(),"resource_directory_utf16":self.resource_directory,"auxiliary_records":self.auxiliary_records.len(),"preload_names":self.preload_names.len()})
    }
}
