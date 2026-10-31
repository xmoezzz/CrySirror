// Copyright (c) 2026 CrySirror contributors. MPL-2.0.
//! Worker-only original service bootstrap. Resource reads use the caller's
//! scoped capability; module bytes are inspected as data and never executed.
use crate::{
    config::Names,
    native_audio::{AudioBindings, AudioSettings, NativeAudioPolicy},
    native_resources::{ImageLookup, ResourceNames},
    native_text::{mugen2_single_byte_map, MessageBindings, MessageProfile, MessageWindow},
    native_ui::{SelectionProfile, SelectionRow},
};
use anyhow::{ensure, Context, Result};
use crysirror_tasks::Cancellation;
use std::sync::Arc;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreferencesOrigin {
    SavedSystemFile,
    OriginalTemplate,
    OriginalConstructor,
}
#[derive(Clone, Debug)]
pub struct NativePreferences {
    pub origin: PreferencesOrigin,
    pub message_speed: usize,
    pub skip_mode: i32,
    pub auto_mode: i32,
    pub window_number: i32,
    pub window_transparency: i32,
    pub movie_volume:i32,
    pub movie_enabled:bool,
    pub launch_count:i32,
    pub audio: AudioSettings,
    /// Untouched original system-file bytes, not a reconstructed save claim.
    pub original: Option<Arc<[u8]>>,
}
impl NativePreferences {
    /// Complete legacy CreateInitData0x4590d0 sets90 volumes, switches1 and
    /// messageSpeed2. GetSystemParam0x534b30 selects these original fields.
    pub fn original_defaults() -> Self {
        Self {
            origin: PreferencesOrigin::OriginalConstructor,
            message_speed: 2,
            skip_mode: 0,
            auto_mode: 0,
            window_number: 0,
            window_transparency: 0,
            movie_volume:90,movie_enabled:true,launch_count:0,
            audio: AudioSettings {
                music_volume: 90,
                sound_volume: 90,
                voice_volume: 90,
                sound_voice_volume: 90,
                music_enabled: true,
                sound_enabled: true,
                voice_enabled: true,
                sound_voice_enabled: true,
            },
            original: None,
        }
    }
    pub fn parse(bytes: &[u8], origin: PreferencesOrigin) -> Result<Self> {
        ensure!(
            (64..=4 * 1024 * 1024).contains(&bytes.len()),
            "NNN system preference byte budget"
        );
        let u32at = |data: &[u8], at: usize| -> Result<u32> {
            Ok(u32::from_le_bytes(
                data.get(at..at + 4)
                    .context("NNN system preference field bounds")?
                    .try_into()?,
            ))
        };
        ensure!(
            u32at(bytes, 0)? == 64 && u32at(bytes, 4)? == 0,
            "NNN native system FILE INFO profile"
        );
        let count = u32at(bytes, 32)?;
        ensure!((1..=10).contains(&count), "NNN native system block count");
        let mut out = Self::original_defaults();
        out.origin = origin;
        out.original = Some(Arc::from(bytes));
        if count > 1 {
            let system = bytes
                .get(64..64 + 2048)
                .context("NNN legacy SYSTEM DATA2048 missing")?;
            ensure!(
                u32at(system, 0)? == 2048 && u32at(system, 4)? == 1,
                "NNN legacy SYSTEM DATA header"
            );
            let field = |offset: usize| -> Result<i32> { Ok(u32at(system, offset)? as i32) };
            out.audio.music_volume = field(176)?;
            out.audio.voice_volume = field(180)?;
            out.audio.sound_volume = field(184)?;
            out.audio.sound_voice_volume = field(212)?;
            out.movie_volume=field(188)?;out.movie_enabled=field(204)?!=0;out.launch_count=field(208)?;
            ensure!((0..=100).contains(&out.movie_volume)&&out.launch_count>=0,"NNN movie volume/launch counter profile");
            out.audio.music_enabled = field(192)? != 0;
            out.audio.voice_enabled = field(196)? != 0;
            out.audio.sound_enabled = field(200)? != 0;
            out.audio.sound_voice_enabled = field(216)? != 0;
            out.message_speed = usize::try_from(field(136)?.clamp(0, 4))?;
            out.skip_mode = field(140)?;
            out.auto_mode = field(164)?;
            out.window_number = field(1920)?;
            out.window_transparency = field(1932)?;
            out.audio.validate()?;
        }
        // Validate every original record boundary even though only preference
        // fields are consumed here. Persistent flag/variable restoration is a
        // separate caller operation, never manufactured from these defaults.
        let mut at = 0usize;
        for _ in 0..count {
            let len = u32at(bytes, at)? as usize;
            ensure!(len >= 32, "NNN system record length");
            at = at.checked_add(len).context("NNN system record overflow")?;
            ensure!(at <= bytes.len(), "NNN truncated system record");
        }
        ensure!(at == bytes.len(), "NNN trailing system preference bytes");
        Ok(out)
    }
}
#[derive(Clone, Debug)]
pub struct NativeWindowSource {
    pub picture: ImageLookup,
    pub rect: [i32; 4],
    pub percent: u16,
}
#[derive(Clone, Debug)]
pub struct NativeWindows {
    pub percent: i32,
    pub body: NativeWindowSource,
    pub name: Option<NativeWindowSource>,
}
#[derive(Clone, Debug)]
pub struct PreparedNativeServices {
    pub preferences: NativePreferences,
    pub audio: NativeAudioPolicy,
    pub music_names: Names,
    pub sound_names: Names,
    pub print_setup: Names,
    pub selection_setup: Names,
    pub window_setup: Names,
    pub name_colors: Names,
    pub character_map: Arc<[char]>,
    pub resources: ResourceNames,
    pub game_setup: Names,
}
fn config(
    read: &mut impl FnMut(&str, u64) -> Result<Option<Vec<u8>>>,
    resources: &ResourceNames,
    name: &str,
    cancel: &Cancellation,
) -> Result<Names> {
    cancel.check()?;
    let encoded = resources.path(&format!("init2/{name}.fxf"))?;
    if let Some(bytes) = read(&encoded, 1 << 20)? {
        ensure!(bytes.len() <= 1 << 20, "NNN config byte budget");
        return Names::parse(&bytes, true);
    }
    let plain = resources.path(&format!("init/{name}.xtx"))?;
    if let Some(bytes) = read(&plain, 1 << 20)? {
        ensure!(bytes.len() <= 1 << 20, "NNN config byte budget");
        return Names::parse(&bytes, false);
    }
    // Constructor defaults are statically reviewed. Absence is explicit data;
    // malformed/inaccessible files are propagated, never treated as missing.
    Ok(Names { tokens: Vec::new() })
}
fn required_names(
    read: &mut impl FnMut(&str, u64) -> Result<Option<Vec<u8>>>,
    resources: &ResourceNames,
    path: &str,
    cancel: &Cancellation,
) -> Result<Names> {
    cancel.check()?;
    let path = resources.path(path)?;
    let bytes = read(&path, 1 << 20)?
        .with_context(|| format!("NNN original service name table missing: {path}"))?;
    ensure!(bytes.len() <= 1 << 20, "NNN service name table byte budget");
    Names::parse(&bytes, false)
}
impl PreparedNativeServices {
    /// Factory convenience: uses the configuration's resource root and both
    /// scoped Asset/Save namespaces. Run on a worker; font aliases are bound
    /// later by message_profile after shared font/window preparation completes.
    pub fn from_configuration(
        configuration: &crate::native_bootstrap::NativeConfiguration,
        module: &[u8],
        storage: &dyn crysirror_core::service::ClientStorage,
        audio_bindings: AudioBindings,
        trial: bool,
        cancel: &Cancellation,
    ) -> Result<Self> {
        use crysirror_core::{
            path::{Mount, VfsPath},
            Error,
        };
        ensure!(
            configuration.module_sha256 == crate::native_bootstrap::MUGEN2_MODULE_SHA256,
            "NNN foreign service configuration module"
        );
        let optional = |mount: Mount, path: &str, limit: u64| -> Result<Option<Vec<u8>>> {
            cancel.check()?;
            let path = VfsPath::new(mount, path)?;
            let source = match if mount == Mount::Asset {
                storage.open_asset(&path)
            } else {
                storage.open_read(&path)
            } {
                Ok(source) => source,
                Err(Error::NotFound) => return Ok(None),
                Err(error) => return Err(error.into()),
            };
            let bytes = crysirror_resource::read_all(source.as_ref(), limit)?;
            cancel.check()?;
            Ok(Some(bytes))
        };
        let saved = optional(
            Mount::Save,
            if trial { "tsysfile.sav" } else { "sysfile.sav" },
            4 << 20,
        )?;
        let mut prepared = Self::mugen2(
            |path, limit| optional(Mount::Asset, &configuration.resources.path(path)?, limit),
            "",
            module,
            &configuration.setup,
            saved.as_deref(),
            audio_bindings,
            trial,
            cancel,
        )?;
        prepared.resources = configuration.resources.clone();
        Ok(prepared)
    }
    /// `saved_system` must come from this game's Save capability. If absent,
    /// native lookup reads sav/sysfile.org before the proven constructor state.
    /// Call after worker admission; no ambient file/device/GPU access occurs.
    pub fn mugen2(
        mut read: impl FnMut(&str, u64) -> Result<Option<Vec<u8>>>,
        root: &str,
        module: &[u8],
        game: &Names,
        saved_system: Option<&[u8]>,
        audio_bindings: AudioBindings,
        trial: bool,
        cancel: &Cancellation,
    ) -> Result<Self> {
        cancel.check()?;
        let resources = ResourceNames::new(root)?;
        let character_map = mugen2_single_byte_map(module)?;
        let preferences = if let Some(bytes) = saved_system {
            NativePreferences::parse(bytes, PreferencesOrigin::SavedSystemFile)?
        } else {
            let template = resources.path(if trial {
                "sav/tsysfile.org"
            } else {
                "sav/sysfile.org"
            })?;
            if let Some(bytes) = read(&template, 4 << 20)? {
                NativePreferences::parse(&bytes, PreferencesOrigin::OriginalTemplate)?
            } else {
                NativePreferences::original_defaults()
            }
        };
        // Original Game GetSystemParam104/204 has a legacy routing switch.
        let mut effective_audio = preferences.audio;
        match game.integer(b"soundVoiceVolumeNotUse")?.unwrap_or(1) {
            0 => {}
            1 => {
                effective_audio.sound_voice_volume = preferences.audio.voice_volume;
                effective_audio.sound_voice_enabled = preferences.audio.voice_enabled;
            }
            _ => {
                effective_audio.sound_voice_volume = preferences.audio.sound_volume;
                effective_audio.sound_voice_enabled = preferences.audio.voice_enabled;
            }
        }
        let audio = NativeAudioPolicy::new(effective_audio, audio_bindings)?;
        let music_names = if game.integer(b"noBGMFlag")?.unwrap_or(0) == 0 {
            required_names(&mut read, &resources, "spt/bgmlist.xtx", cancel)?
        } else {
            Names { tokens: vec![] }
        };
        let sound_names = required_names(&mut read, &resources, "spt/selist.xtx", cancel)?;
        ensure!(
            music_names.tokens.len() % 2 == 0 && sound_names.tokens.len() % 2 == 0,
            "NNN native audio table pairs"
        );
        let print_setup = config(&mut read, &resources, "PrintMessage", cancel)?;
        let selection_setup = config(&mut read, &resources, "SelectMessage", cancel)?;
        let window_setup = config(&mut read, &resources, "messageWindow", cancel)?;
        let color_path = print_setup
            .parameter(b"nameColorFileName")
            .unwrap_or(b"spt/namecol.xtx");
        let path = resources.encoded_path(color_path)?;
        let name_colors = if print_setup.integer(b"nameColorChangeFlag")?.unwrap_or(1) != 0
            || print_setup.integer(b"autoNameWindowFlag")?.unwrap_or(0) != 0
        {
            let bytes = read(&path, 1 << 20)?
                .with_context(|| format!("NNN native name-color file missing: {path}"))?;
            Names::parse(&bytes, false)?
        } else {
            Names { tokens: vec![] }
        };
        cancel.check()?;
        Ok(Self {
            preferences,
            audio,
            music_names,
            sound_names,
            print_setup,
            selection_setup,
            window_setup,
            name_colors,
            character_map,
            resources,
            game_setup: game.clone(),
        })
    }
    pub fn message_profile(
        &self,
        bindings: MessageBindings,
        font: &str,
        advance_buttons: u64,
    ) -> Result<MessageProfile> {
        self.message_profile_with_windows(bindings, font, advance_buttons, None)
    }
    pub fn message_profile_with_windows(
        &self,
        bindings: MessageBindings,
        font: &str,
        advance_buttons: u64,
        name_window: Option<MessageWindow>,
    ) -> Result<MessageProfile> {
        ensure!(
            self.preferences.auto_mode == 0,
            "NNN saved auto-message policy requires native voice/auto timing mapping"
        );
        if self
            .game_setup
            .integer(b"noMessageWindowFlag")?
            .unwrap_or(0)
            == 0
        {
            ensure!(
                bindings.window.is_some(),
                "NNN original message-window image not prepared"
            );
            if self.window_setup.integer(b"nameWindowFlag")?.unwrap_or(0) != 0 {
                ensure!(
                    name_window.is_some(),
                    "NNN original name-window image not prepared"
                );
            }
        }
        let mut profile = MessageProfile::from_legacy_setup(
            &self.print_setup,
            bindings,
            font,
            self.preferences.message_speed,
            Some(&self.name_colors),
            Some(self.character_map.clone()),
            advance_buttons,
        )?;
        profile.name_window = name_window;
        profile.validate()?;
        Ok(profile)
    }
    pub fn windows(&self, script_window: i32, pack_levels: u32) -> Result<Option<NativeWindows>> {
        if self
            .game_setup
            .integer(b"noMessageWindowFlag")?
            .unwrap_or(0)
            != 0
        {
            return Ok(None);
        }

        let n = |key: &str, default: i32| -> Result<i32> {
            Ok(self
                .window_setup
                .integer(key.as_bytes())?
                .unwrap_or(default))
        };
        let config_count = n("windowNumberConfig", 1)?;
        let script_count = n("windowNumberScript", 1)?;
        let conf = self.preferences.window_number;
        ensure!(
            (1..=64).contains(&config_count)
                && (1..=64).contains(&script_count)
                && conf >= 0
                && conf < config_count
                && script_window >= 0
                && script_window < script_count,
            "NNN original window variant bounds"
        );
        let key = |base: &str| {
            if config_count > 1 && script_count > 1 {
                format!("{base}{}_{}", script_window + 1, conf + 1)
            } else {
                format!(
                    "{base}{}",
                    if script_count > 1 {
                        script_window + 1
                    } else {
                        conf + 1
                    }
                )
            }
        };
        let picture = |base: &str, default: &[u8]| -> Result<ImageLookup> {
            let value = self
                .window_setup
                .parameter(key(base).as_bytes())
                .or_else(|| self.window_setup.parameter(base.as_bytes()))
                .unwrap_or(default);
            let mut path = b"sys/".to_vec();
            path.extend(value);
            self.resources.picture(&path, pack_levels)
        };
        let body = NativeWindowSource {
            percent: (100 - self.preferences.window_transparency).clamp(0, 100) as u16,
            picture: picture("filenameWindow", b"ta_meswin")?,
            rect: [
                n("printX", 0)?,
                n("printY", 300)?,
                n("sizeX", 640)?,
                n("sizeY", 180)?,
            ],
        };
        let name = if n("nameWindowFlag", 0)? != 0 {
            Some(NativeWindowSource {
                percent: (100 - self.preferences.window_transparency).clamp(0, 100) as u16,
                picture: picture("filenameNameWindow", b"ta_namewin")?,
                rect: [
                    n("namePrintX", 0)?,
                    n("namePrintY", 200)?,
                    n("nameSizeX", 640)?,
                    n("nameSizeY", 100)?,
                ],
            })
        } else {
            None
        };
        Ok(Some(NativeWindows {
            percent: (100 - self.preferences.window_transparency).clamp(0, 100),
            body,
            name,
        }))
    }
    /// Owner construction performs no resource/font I/O. Choice counts are
    /// taken from each exact pending original SELECT instruction.
    pub fn selection_bridge(
        &self,
        bindings: SelectionProfile,
    ) -> Result<crate::native_ui::NativeSelectionBridge> {
        let services = self.clone();
        let base = bindings.clone();
        crate::native_ui::NativeSelectionBridge::new_dynamic(
            bindings,
            Arc::new(move |choices, messages| {
                services.selection_profile(base.clone(), choices, messages)
            }),
        )
    }
    /// Fixed legacy rows are proven by PrintMessageSub0x4fb3f0. Scroll,
    /// appearance effects and timer/special selection remain explicit fences.
    pub fn selection_profile(
        &self,
        base: SelectionProfile,
        choices: usize,
        messages: usize,
    ) -> Result<SelectionProfile> {
        let n = |key: &str, default: i32| -> Result<i32> {
            Ok(self
                .selection_setup
                .integer(key.as_bytes())?
                .unwrap_or(default))
        };
        ensure!(
            n("useScrollFlag", 0)? == 0 && n("firstAppearType", 0)? == 0,
            "NNN scrolling/appearance selection profile unsupported"
        );
        ensure!(
            choices > 0
                && choices <= usize::try_from(n("messagePrintGyo", 4)?)?
                && choices + messages <= 64,
            "NNN fixed selection row count"
        );
        let x = n("messagePrintX", 66)?;
        let y = n("messagePrintY", 383)?;
        let step = n("messageNextY", 30)?;
        let dx = n("cursorDeltaX", -8)?;
        let dy = n("cursorDeltaY", -2)?;
        let w = n("cursorSizeX", 469)?;
        let h = n("cursorSizeY", 27)?;
        let mut p = base;
        p.face.size = u16::try_from(n("fontSize", 24)?)?;
        p.face.spacing = 1;
        p.face.color = [255; 4];
        p.single_byte_map = Some(self.character_map.clone());
        p.message_origins = (0..messages).map(|i| [x, y + i as i32 * step]).collect();
        p.rows = (0..choices)
            .map(|i| {
                let row = y + (messages + i) as i32 * step;
                SelectionRow {
                    text_origin: [x, row],
                    hit_rect: [x + dx, row + dy, w, h],
                }
            })
            .collect();
        p.validate()?;
        Ok(p)
    }
}
/// Convert a worker-resolved engine asset into an ordinary retained window
/// binding. Original image lookup/decoding happens before this owner resource.
pub fn resolved_window(
    id: u32,
    source: &NativeWindowSource,
    asset: String,
    source_size: [u32; 2],
) -> Result<MessageWindow> {
    ensure!(
        source.rect[2] > 0
            && source.rect[3] > 0
            && source_size[0] >= source.rect[2] as u32
            && source_size[1] >= source.rect[3] as u32,
        "NNN original window source crop"
    );
    Ok(MessageWindow {
        id,
        asset,
        rect: source.rect,
        source_size,
        percent: source.percent,
        resource: None,
    })
}
/// Resource coverage has already been normalized by shared SpriteAlpha when
/// native metadata selected colour-key drawing. Per-mask resources retain it.
pub fn resolved_window_resource(
    id: u32,
    source: &NativeWindowSource,
    asset: String,
    image: Arc<dyn crysirror_core::image_processing::ImageResource>,
) -> Result<MessageWindow> {
    let mut window = resolved_window(id, source, asset, image.size())?;
    window.resource = Some(image);
    Ok(window)
}
