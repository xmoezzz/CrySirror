// SPDX-License-Identifier: MPL-2.0
//! The standalone level-19 StaffRollScroll, StaffRollMgr and image animation.
use super::*;
use crate::{tjs::TjsState, unity_scene::ObjectKey};

pub(super) struct Roll {
    pub duration: u64,
    pub bgm: String,
    pictures: Vec<ObjectKey>,
    rows: Vec<(Widget, f32)>,
    stop: f32,
    base: usize,
    picture: Option<usize>,
}

impl Roll {
    pub fn open(
        s: &mut TjsState,
        widgets: &mut Vec<Widget>,
        assets: &mut Assets,
        op: &mut Opener,
    ) -> Result<Self> {
        let config = s.retail.table("staffroll_data.nei")?;
        let duration = u64::try_from(config.int(1, 2))? * 1000;
        ensure!(duration > 0, "invalid staff roll duration");
        let bgm = format!("{}.ogg", config.string(1, 1));
        let records = s.retail.table("staffroll_stafflist.nei")?;
        let templates: Vec<_> = widgets
            .extract_if(.., |w| w.path.contains("/StaffRollUnitParent/"))
            .collect();
        let mut rows = Vec::new();
        let mut stop = 0.;
        let mut y = 0.;
        for row in 1..records.rows {
            let flags = records.string(0, row);
            let has = |s: &str| flags.split(',').any(|v| v == s);
            if has("ご主人様") && s.player.flag("__lockUserDraftMaid") != 0 {
                continue;
            }
            let mut name = records.string(1, row);
            if has("コピーライト") {
                name.insert(0, '©');
            }
            let template = if has("ロゴ") {
                "Text_Logo"
            } else if name.is_empty() || has("文字_小") {
                "Text_Small1"
            } else if has("文字_小2") {
                "Text_Small2"
            } else {
                "Text"
            };
            let root = templates
                .iter()
                .find(|w| w.path.rsplit('/').next() == Some(template))
                .context("missing level 19 credit template")?;
            let h = root.rect[3];
            let center = y + h / 2. + assets.credits_pad_y;
            if has("停止位置") || (stop == 0. && row + 1 == records.rows) {
                stop = center.trunc();
            }
            for t in templates.iter().filter(|w| {
                w.path == root.path
                    || w.path
                        .strip_prefix(&root.path)
                        .is_some_and(|s| s.starts_with('/'))
            }) {
                let mut w = t.clone();
                if let Some(label) = &mut w.label {
                    if has("ロゴ") {
                        continue;
                    }
                    label.set_text(&name);
                }
                let top = 540. + center - h / 2. + t.rect[1] - root.rect[1];
                rows.push((w, top));
            }
            y += h + 2. * assets.credits_pad_y;
        }
        let pictures = assets
            .unity
            .resources_under(op, "SceneStaffRoll/Texture", 28)?;
        ensure!(
            !pictures.is_empty() && stop > 0.,
            "incomplete staff roll data"
        );
        Ok(Self {
            duration,
            bgm,
            pictures,
            rows,
            stop,
            base: widgets.len(),
            picture: None,
        })
    }
    pub fn frame(
        &mut self,
        ms: u64,
        ws: &mut Vec<Widget>,
        a: &mut Assets,
        op: &mut Opener,
        host: &mut dyn HostServices,
        c: ClientId,
    ) -> Result<()> {
        ws.truncate(self.base);
        let progress = (ms as f64 / self.duration as f64).min(1.);
        let shift = self.stop * progress as f32;
        let intervals = self.pictures.len() - 1;
        let index = (intervals as f64 * progress) as usize;
        let since = if intervals == 0 {
            0.
        } else {
            (ms as f64 - self.duration as f64 * index as f64 / intervals as f64) as f32 / 1000.
        };
        // Animation_Staffroll_Image_Apply raises ApplyTex at frame 50 (60 Hz).
        let visible = if index == 0 {
            None
        } else if since >= 50. / 60. {
            Some(index)
        } else {
            index.checked_sub(1).filter(|i| *i > 0)
        };
        let texture = if visible != self.picture {
            visible
                .map(|i| {
                    a.texture(op, host, c, self.pictures[i].clone())
                        .map(|t| t.0)
                })
                .transpose()?
        } else {
            None
        };
        self.picture = visible;
        let smooth = |t: f32| {
            let t = t.clamp(0., 1.);
            t * t * (3. - 2. * t)
        };
        let alpha = if since < 0.75 {
            1. - smooth(since / 0.75) * (254. / 255.)
        } else {
            1. / 255. + smooth((since - 55. / 60.) / 0.6) * (254. / 255.)
        };
        for w in ws.iter_mut() {
            if w.path.ends_with("/Texture") {
                if let Some(key) = &texture {
                    w.asset = key.clone();
                }
                w.tint[3] = if visible.is_some() {
                    (255. * alpha).round() as u8
                } else {
                    0
                };
            } else if w.path.ends_with("/Texture/Sprite") {
                w.tint[3] = if visible.is_some() { 117 } else { 0 };
            } else if w.path.ends_with("/Logo") {
                let time = ms.saturating_sub(self.duration) as f32 / 1000.;
                w.tint[3] = if ms >= self.duration {
                    ((0.003 + 0.997 * smooth((time - 179. / 60.) / 2.05)) * 255.).round() as u8
                } else {
                    0
                };
            }
        }
        for (w, top) in &self.rows {
            let y = top - shift;
            if y + w.rect[3] > 0. && y < 1080. {
                let mut w = w.clone();
                w.rect[1] = y;
                ws.push(w);
            }
        }
        ws[self.base..].sort_by_key(|w| w.depth);
        Ok(())
    }
}

#[test]
#[ignore = "requires installed level 19 and the retail scripts and sound"]
fn installed_credits_callers_scroll_and_return() -> Result<()> {
    let packs = ["sound", "sound2"];
    let (_tmp, mut rt, c, mut op, files, _, _) = daily::installed_panel(&packs)?;
    let mut vm = Vm {
        linked_cm3d2: None,
        client: c,
        config: Config {
            data_dir: op.dir.clone(),
            archives: ["csv", "system", "system2", "script"]
                .into_iter()
                .chain(packs)
                .map(str::to_owned)
                .collect(),
        },
        game: None,
    };
    vm.advance(&mut rt.host, InputFrame::default())?;
    let game = vm.game.as_mut().unwrap();
    {
        let mut t = game.script.tjs.lock().unwrap();
        let s = &mut t.runtime.host_mut().state;
        s.player.schedule_slot.resize_with(40, Default::default);
        crate::tjs::retail::operations::start_day(s)?;
    }
    for (file, period) in [
        ("daytimemain.ks", daily::Period::Daytime),
        ("nightmain.ks", daily::Period::Night),
    ] {
        let label = "*スタッフロール";
        game.script.load(&files, op.storage.as_ref(), file, label)?;
        game.script.enabled = true;
        game.script.exec();
        game.enter(Scene::Script, &mut op, &mut rt.host, c, 0)?;
        for _ in 0..32 {
            rt.tick_control(250, InputFrame::default())?;
            game.update(&mut rt.host, c, &mut op, InputFrame::default())?;
            if game.scene == Scene::Credits {
                break;
            }
        }
        assert_eq!(
            game.scene,
            Scene::Credits,
            "{file} {}",
            game.script.position()
        );

        let roll = game.credits.as_ref().unwrap();
        assert_eq!(roll.duration, 190000);
        assert!(roll.rows.len() >= 250);
        for _ in 0..80 {
            rt.tick_control(250, InputFrame::default())?;
            game.update(&mut rt.host, c, &mut op, InputFrame::default())?;
        }
        assert!(
            game.widgets
                .iter()
                .any(|w| w.label.as_ref().is_some_and(|l| !l.text.is_empty()))
        );
        assert!(
            game.widgets
                .iter()
                .any(|w| w.path.ends_with("/Texture") && !w.asset.is_empty() && w.tint[3] > 0)
        );
        if period == daily::Period::Daytime {
            let input = InputFrame {
                buttons_pressed: buttons::ENTER,
                buttons_down: buttons::ENTER,
                ..Default::default()
            };
            game.update(&mut rt.host, c, &mut op, input)?;
            assert!(game.leaving.is_none());
            game.update(&mut rt.host, c, &mut op, InputFrame::default())?;
        } else {
            let elapsed = rt.host.now_ms().saturating_sub(game.entered);
            rt.tick_control(199000 - elapsed - 1, InputFrame::default())?;
            game.update(&mut rt.host, c, &mut op, InputFrame::default())?;
            assert!(game.leaving.is_none());
            assert!(
                game.widgets
                    .iter()
                    .any(|w| w.path.ends_with("/Logo") && w.tint[3] == 255)
            );
            rt.tick_control(1, InputFrame::default())?;
            game.update(&mut rt.host, c, &mut op, InputFrame::default())?;
        }
        assert!(game.leaving.is_some());
        rt.tick_control(1000, InputFrame::default())?;
        game.update(&mut rt.host, c, &mut op, InputFrame::default())?;
        assert_eq!(game.scene, Scene::Script);
        for _ in 0..32 {
            rt.tick_control(250, InputFrame::default())?;
            game.update(&mut rt.host, c, &mut op, InputFrame::default())?;
            if game.scene == Scene::Daily(period) {
                break;
            }
        }
        assert_eq!(game.scene, Scene::Daily(period));
    }
    Ok(())
}
