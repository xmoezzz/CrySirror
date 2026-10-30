//! Native built-in 1e/51 policy. Retained text entry, pointer targeting,
//! dialogs and frozen paints belong to the shared engine, never this module.
use super::*;

// These resources/controls are adapter-owned, outside native bitmap slots0..63.
const BACKGROUND: u8 = 250;
const CHIP: u8 = 251;
const ROOT: u32 = 0x3800_0000;
const BUTTON_IMAGE: u32 = ROOT + 2;
const WIDGET_ROOT: u32 = ROOT;
const NAME: u32 = ROOT + 1;
const MAIL: u32 = ROOT + 2;
const SUBMIT: u32 = ROOT + 3;
const DIALOG: u32 = 51;
const CANVAS: u32 = 0x3300_0001;
const PRESENTATION: u32 = 0x1400_0003;
pub(super) fn clear_capture(host: &mut dyn HostServices, client: ClientId) -> Result<()> {
    host.submit_engine(client,1,&serde_json::to_vec(&json!({"canvases":[{"op":"hide","layer":PRESENTATION},{"op":"clear","id":CANVAS,"color":[0,0,0,0]}]}))?)?;
    Ok(())
}

/// Portable client-local equivalent of native CLEARNAME/CLEARMAIL registry
/// values. This is user preference storage, not an original game save format.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProfileData {
    schema: u32,
    #[serde(rename = "CLEARNAME")]
    name: String,
    #[serde(rename = "CLEARMAIL")]
    mail: String,
}
impl Default for ProfileData {
    fn default() -> Self {
        Self {
            schema: 1,
            name: String::new(),
            mail: String::new(),
        }
    }
}
impl ProfileData {
    pub(super) fn validate(&self) -> Result<()> {
        ensure!(self.schema == 1, "Circus local profile schema");
        for value in [&self.name, &self.mail] {
            let (encoded, _, errors) = encoding_rs::SHIFT_JIS.encode(value);
            ensure!(
                !errors && !value.chars().any(char::is_control) && encoded.len() <= 254,
                "Circus native edit requires at most 254 Shift-JIS bytes"
            );
        }
        Ok(())
    }
}
#[derive(Clone, Copy)]
enum Confirmation {
    Empty,
    InvalidMail,
}
pub(super) struct ProfileInput {
    data: ProfileData,
    opened: bool,
    hovered: bool,
    sound: Option<String>,
    submitted: Option<ProfileData>,
    confirmation: Option<Confirmation>,
    accepted: bool,
    captured: bool,
}
impl Bridge<'_> {
    pub(super) fn player_input(&mut self) -> Result<bool> {
        if self.services.profile.is_none() {
            let Some(Prepared::Profile(data)) =
                self.services.prepare(Work::ProfileLoad, self.client)?
            else {
                return Ok(false);
            };
            self.services.profile = Some(ProfileInput {
                data,
                opened: false,
                hovered: false,
                sound: None,
                submitted: None,
                confirmation: None,
                accepted: false,
                captured: false,
            });
        }
        let mut state = self
            .services
            .profile
            .take()
            .context("Circus profile state")?;
        let result = self.tick_player_input(&mut state);
        if !matches!(result, Ok(true)) {
            self.services.profile = Some(state);
        }
        result
    }
    fn tick_player_input(&mut self, state: &mut ProfileInput) -> Result<bool> {
        for (slot, name) in [
            (BACKGROUND, "system\\playerinput"),
            (CHIP, "system\\playerinput_chip"),
        ] {
            if !self.emit(&Event::LoadImage {
                slot,
                name: name.into(),
            })? {
                return Ok(false);
            }
        }
        if !state.opened {
            let image = self
                .services
                .images
                .get(&BACKGROUND)
                .context("Circus profile background")?;
            let [w, h] = self.services.image_extents[&BACKGROUND];
            let layers = vec![
                json!({"op":"put","layer":{"id":ROOT,"rect":[0,0,1280,720],"paint":{"kind":"solid","color":[0,0,0,0]},"order":[1900000000,0]}}),
                json!({"op":"put","layer":{"id":ROOT+1,"parent":ROOT,"rect":[0,0,w,h],"paint":{"kind":"image","asset":image.path.relative()},"uv":[0.,0.,w as f32/image.size[0] as f32,h as f32/image.size[1] as f32],"order":[1900000000,1]}}),
                self.profile_button_image(false)?,
            ];
            let ui = vec![
                json!({"op":"put","widget":{"id":WIDGET_ROOT,"rect":[0,0,1280,720],"color":[0,0,0,0],"modal":true,"clip_children":true}}),
                json!({"op":"put","widget":{"id":NAME,"parent":WIDGET_ROOT,"rect":[466,356,407,43],"kind":"text_field","text":state.data.name,"color":[255,255,255,255],"text_color":[0,0,0,255],"text_inset":[2,1],"font":self.font,"text_size":36,"max_length":254,"z":1}}),
                json!({"op":"put","widget":{"id":MAIL,"parent":WIDGET_ROOT,"rect":[466,456,407,43],"kind":"text_field","text":state.data.mail,"color":[255,255,255,255],"text_color":[0,0,0,255],"text_inset":[2,1],"font":self.font,"text_size":36,"max_length":254,"z":2}}),
                // Native hit region is inclusive through x1253/y671, while
                // its visual chip is only205x45. Shared UI owns hit testing.
                json!({"op":"put","widget":{"id":SUBMIT,"parent":WIDGET_ROOT,"rect":[1036,626,218,46],"kind":"button","activate_on_press":true,"interaction_only":true,"keyboard_focusable":false,"color":[0,0,0,0],"z":3}}),
                json!({"op":"focus","id":NAME}),
            ];
            self.submit(layers, ui)?;
            state.opened = true;
            return Ok(false);
        }
        if let Some(name) = state.sound.clone() {
            if !self.emit(&Event::PlayAudio {
                channel: if name.eq_ignore_ascii_case("se\\201.pcm") {
                    7
                } else {
                    6
                },
                name,
                voice: false,
            })? {
                return Ok(false);
            }
            state.sound = None;
        }
        if state.accepted {
            if !state.captured {
                self.capture_player_input()?;
                state.captured = true;
            }
            let Some(prepared) = self
                .services
                .prepare(Work::ProfileSave(state.data.clone()), self.client)?
            else {
                return Ok(false);
            };
            ensure!(
                matches!(prepared, Prepared::ProfileSaved),
                "Circus profile write acknowledgement"
            );
            return Ok(true);
        }
        let events: Vec<Value> =
            serde_json::from_slice(&self.host.query_engine(self.client, EngineQuery::UiEvents)?)?;
        for event in events {
            if event["kind"] == "dialog_result"
                && event["widget"]["id"].as_u64() == Some(u64::from(DIALOG))
            {
                let confirmation = state
                    .confirmation
                    .take()
                    .context("Circus profile dialog owner")?;
                if matches!(confirmation, Confirmation::Empty)
                    && event["value"].as_f64() == Some(6.)
                {
                    state.accepted = true;
                }
            } else if event["kind"] == "activate"
                && event["widget"]["id"].as_u64() == Some(u64::from(SUBMIT))
            {
                let name: Value = serde_json::from_slice(
                    &self
                        .host
                        .query_engine(self.client, EngineQuery::UiWidget(NAME))?,
                )?;
                let mail: Value = serde_json::from_slice(
                    &self
                        .host
                        .query_engine(self.client, EngineQuery::UiWidget(MAIL))?,
                )?;
                state.submitted = Some(ProfileData {
                    schema: 1,
                    name: name["text"]
                        .as_str()
                        .context("Circus name control missing")?
                        .into(),
                    mail: mail["text"]
                        .as_str()
                        .context("Circus mail control missing")?
                        .into(),
                });
                state.sound = Some("se\\201.pcm".into());
            }
        }
        if let Some(data) = state.submitted.take() {
            data.validate()?;
            if data.name.is_empty() || data.mail.is_empty() {
                state.data = data;
                self.profile_dialog(Confirmation::Empty)?;
                state.confirmation = Some(Confirmation::Empty);
            } else if !(data.mail.contains('@') && data.mail.contains('.')) {
                self.profile_dialog(Confirmation::InvalidMail)?;
                state.confirmation = Some(Confirmation::InvalidMail);
            } else {
                state.data = data;
                state.accepted = true;
            }
        }
        if state.confirmation.is_none() {
            let interaction: Value = serde_json::from_slice(
                &self
                    .host
                    .query_engine(self.client, EngineQuery::UiInteraction(SUBMIT))?,
            )?;
            let hovered = interaction["hovered"]
                .as_bool()
                .context("Circus submit interaction missing")?;
            if hovered != state.hovered {
                self.submit(vec![self.profile_button_image(hovered)?], vec![])?;
                if hovered && state.sound.is_none() {
                    state.sound = Some("se\\200.pcm".into());
                }
                state.hovered = hovered;
            }
        }
        Ok(false)
    }
    fn profile_button_image(&self, hovered: bool) -> Result<Value> {
        let image = self
            .services
            .images
            .get(&CHIP)
            .context("Circus profile chip")?;
        // Native4502C0 places the 205x90 bitmap, then4500E0 clips
        // the destination. Moving it up45 exposes the second state.
        let y = if hovered { 45. } else { 0. };
        Ok(
            json!({"op":"put","layer":{"id":BUTTON_IMAGE,"parent":ROOT,"rect":[1036,626,205,45],"paint":{"kind":"image","asset":image.path.relative()},"uv":[0.,y/image.size[1] as f32,205./image.size[0] as f32,(y+45.)/image.size[1] as f32],"order":[1900000000,2]}}),
        )
    }
    fn profile_dialog(&mut self, confirmation: Confirmation) -> Result<()> {
        let (message, buttons, default) = match confirmation {
            Confirmation::Empty => (
                "Some fields are empty. Continue?",
                json!([{"id":6,"label":"Yes"},{"id":7,"label":"No"}]),
                6,
            ),
            Confirmation::InvalidMail => (
                "Enter an email address containing @ and .",
                json!([{"id":1,"label":"OK"}]),
                1,
            ),
        };
        self.submit(vec![],vec![json!({"op":"open_dialog","dialog":{"id":DIALOG,"title":"D.C.4 Plus Harmony","message":message,"buttons":buttons,"default":default,"font":self.font}})])
    }
    fn capture_player_input(&mut self) -> Result<()> {
        // Native41AE60->425EE0 freezes the current form as scene resource45.
        // UI paints/fonts/clips and canvas copying remain common capabilities.
        let widgets: Vec<Value> = serde_json::from_slice(&self.host.query_engine(
            self.client,
            EngineQuery::UiSnapshot {
                canvas: [1280, 720],
                root: Some(WIDGET_ROOT),
            },
        )?)?;
        let mut canvases = vec![
            json!({"op":"create","id":CANVAS,"width":1280,"height":720}),
            json!({"op":"clear","id":CANVAS,"color":[0,0,0,255]}),
            json!({"op":"copy_subtree","source":ROOT,"destination":CANVAS,"rect":[0,0,1280,720],"position":[0,0]}),
        ];
        for layer in widgets {
            canvases.push(json!({"op":"draw","id":CANVAS,"layer":layer}));
        }
        canvases.push(json!({"op":"present","id":CANVAS,"layer":PRESENTATION,"parent":SCENE_ROOT,"order":0,"position":[0,0]}));
        self.host.submit_engine(
            self.client,
            1,
            &serde_json::to_vec(&json!({"canvases":canvases}))?,
        )?;
        self.submit(
            vec![
                json!({"op":"remove","id":ROOT}),
                json!({"op":"set_visible","id":SCENE_ROOT,"visible":true}),
                json!({"op":"set_visible","id":COLOUR_ROOT,"visible":false}),
            ],
            vec![json!({"op":"remove","id":WIDGET_ROOT})],
        )?;
        self.services.profile_captured = true;
        Ok(())
    }
}
