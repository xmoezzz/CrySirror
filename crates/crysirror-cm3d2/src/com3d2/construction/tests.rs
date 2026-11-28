// SPDX-License-Identifier: MPL-2.0
use super::*;
use crate::tjs::retail::operations;

#[test]
#[ignore = "requires COM3D2_ROOT, temporary save root"]
fn installed_construction_controls_and_second_day() -> Result<()> {
    let (_tmp, mut rt, c, mut op, _, mut s, mut assets) = daily::installed_panel(&[])?;
    operations::start_day(&mut s)?;
    s.player.money = 500_000;
    let mut screen = Screen::load(&mut s, &mut assets, &mut op, &mut rt.host, c)?;
    let mut widgets = screen.widgets(&mut s, &mut assets, &mut op, &mut rt.host, c)?;
    let mut now = 0;
    let mut click = |action: Option<Action>,
                     ok: bool,
                     screen: &mut Screen,
                     widgets: &mut Vec<Widget>,
                     s: &mut TjsState|
     -> Result<bool> {
        let rect = if let Some(a) = action {
            screen
                .bindings
                .iter()
                .find(|(_, act)| *act == a)
                .map(|(r, _)| *r)
                .with_context(|| format!("no control {a:?}"))?
        } else {
            let start = screen
                .dialog
                .as_ref()
                .context("no construction dialog")?
                .first;
            widgets[start..]
                .iter()
                .find(|w| {
                    w.path
                        .ends_with(if ok { "/Base/Ok" } else { "/Base/Cancel" })
                })
                .and_then(|w| w.collider)
                .context("missing modal button")?
        };
        let mut input = InputFrame {
            pointer_x: (rect[0] + rect[2] / 2.) as i32,
            pointer_y: (rect[1] + rect[3] / 2.) as i32,
            buttons_pressed: buttons::MOUSE_LEFT,
            ..Default::default()
        };
        ensure!(
            (0..1920).contains(&input.pointer_x) && (0..1080).contains(&input.pointer_y),
            "offscreen {action:?} {rect:?}"
        );
        screen.input(
            s,
            widgets,
            &mut assets,
            &mut op,
            &mut rt.host,
            c,
            input,
            0,
            now,
        )?;
        input.buttons_pressed = 0;
        let closed = screen.input(
            s,
            widgets,
            &mut assets,
            &mut op,
            &mut rt.host,
            c,
            input,
            buttons::MOUSE_LEFT,
            now + 20,
        )?;
        if !closed {
            screen.input(
                s,
                widgets,
                &mut assets,
                &mut op,
                &mut rt.host,
                c,
                input,
                0,
                now + 500,
            )?;
        }
        now += 510;
        Ok(closed)
    };
    // The training room in slot zero is fixed. A blocked replacement acknowledges and returns.
    click(Some(Action::Place), true, &mut screen, &mut widgets, &mut s)?;
    assert!(matches!(screen.confirm, Some(Confirm::Notice)));
    click(None, true, &mut screen, &mut widgets, &mut s)?;
    for (action, ok) in [
        (Some(Action::Slot(8)), true),
        (Some(Action::Place), true),
        (None, true),
        (Some(Action::Type(120)), true),
        (Some(Action::Ok), true),
        (None, false),
    ] {
        click(action, ok, &mut screen, &mut widgets, &mut s)?;
    }
    assert!(s.retail.construction.is_empty());
    for (action, ok) in [(Some(Action::Ok), true), (None, true), (None, true)] {
        click(action, ok, &mut screen, &mut widgets, &mut s)?;
    }
    assert_eq!(Catalog::effective(&s.retail, 8), Some(120));
    assert_eq!(s.player.money, 500_000);
    assert_eq!(Catalog::live(&s.retail, 8), None);
    assert!(widgets.iter().any(|w| w.path.ends_with("/Text Number Cost")
        && w.label.as_ref().is_some_and(|l| l.text == "500,000")));
    assert!(widgets.iter().any(|w| w.path.ends_with("/Image Change")));
    // Both read-only tabs must be reachable from native menu hit targets.
    click(
        Some(Action::ShowTab(Tab::Current)),
        true,
        &mut screen,
        &mut widgets,
        &mut s,
    )?;
    assert_eq!(screen.tab, Tab::Current);
    assert!(
        !screen
            .bindings
            .iter()
            .any(|(_, a)| *a == Action::InspectSlot(8))
    );
    assert!(
        !widgets
            .iter()
            .any(|w| w.path.starts_with("/Window Money Parameter"))
    );
    click(
        Some(Action::InspectSlot(0)),
        true,
        &mut screen,
        &mut widgets,
        &mut s,
    )?;
    assert!(
        widgets
            .iter()
            .any(|w| w.path.starts_with(MINI) && w.path.contains("Text Status"))
    );
    assert!(
        !widgets
            .iter()
            .any(|w| w.path.starts_with(MINI) && w.path.contains("Text Number of cost"))
    );
    click(
        Some(Action::ShowTab(Tab::Catalog)),
        true,
        &mut screen,
        &mut widgets,
        &mut s,
    )?;
    assert!(!widgets.iter().any(|w| w.path.starts_with(MINI)));
    click(
        Some(Action::Type(120)),
        true,
        &mut screen,
        &mut widgets,
        &mut s,
    )?;
    assert!(widgets.iter().any(|w| w.path.starts_with(MINI)
        && w.path.ends_with("Text Number of cost")
        && w.label.as_ref().is_some_and(|l| l.text == "500000")));
    assert!(
        widgets
            .iter()
            .any(|w| w.path.ends_with("/Text Recipe Number")
                && w.label.as_ref().is_some_and(|l| l.text == "RECIPE 1"))
    );
    assert!(
        widgets
            .iter()
            .any(|w| w.path.ends_with("/Text Category Number")
                && w.label.as_ref().is_some_and(|l| l.text == "CATEGORY 1"))
    );
    assert!(
        !screen
            .bindings
            .iter()
            .any(|(_, a)| matches!(a, Action::Place | Action::Destroy))
    );
    assert_eq!(s.player.money, 500_000);
    assert!(click(
        Some(Action::Ok),
        true,
        &mut screen,
        &mut widgets,
        &mut s
    )?);
    operations::start_day(&mut s)?;
    assert_eq!((s.player.days, s.player.money), (2, 0));
    assert_eq!(Catalog::live(&s.retail, 8), Some(120));
    assert_eq!(Catalog::live(&s.retail, 2), None);
    assert!(s.retail.construction.is_empty());
    assert_eq!(s.retail.peak_facilities, 3);
    Ok(())
}

#[test]
#[ignore = "requires COM3D2_ROOT, temporary save root"]
fn installed_construction_replacement_demolition_and_lost_funds() -> Result<()> {
    let (_tmp, _, _, _, _, mut s, _) = daily::installed_panel(&[])?;
    operations::start_day(&mut s)?;
    let cat = Catalog::load(&mut s.retail)?;
    s.player.money = 2_000_000;
    cat.reserve(&mut s, 9, Some(120))?;
    cat.reserve(&mut s, 5, Some(130))?;
    cat.reserve(&mut s, 9, Some(160))?;
    assert_eq!(cat.total(&s.retail, None)?, 1_750_000);
    s.player.money = 1_000_000;
    operations::start_day(&mut s)?;
    assert_eq!(s.player.money, 250_000);
    assert_eq!(Catalog::live(&s.retail, 9), Some(160));
    assert_eq!(Catalog::live(&s.retail, 5), None);
    assert!(s.retail.construction.is_empty());
    assert!(cat.reserve(&mut s, 9, Some(130)).is_err());
    cat.reserve(&mut s, 9, None)?;
    assert_eq!(Catalog::live(&s.retail, 9), Some(160));
    operations::start_day(&mut s)?;
    assert_eq!(Catalog::live(&s.retail, 9), None);
    assert_eq!(s.player.money, 250_000);
    assert_eq!(s.retail.peak_facilities, 3);
    assert!(cat.reserve(&mut s, 0, None).is_err());
    Ok(())
}

#[test]
#[ignore = "requires COM3D2_ROOT, detached text-input presenter"]
fn installed_room_name_edits_follow_live_and_reserved_instances() -> Result<()> {
    let (_tmp, mut rt, c, mut op, _, mut s, mut assets) = daily::installed_panel(&[])?;
    operations::start_day(&mut s)?;
    s.player.money = 500_000;
    rt.host.text_input.set_presenter_available(true);
    rt.host.text_input.focus(true);
    let mut screen = Screen::load(&mut s, &mut assets, &mut op, &mut rt.host, c)?;
    screen.catalog.reserve(&mut s, 8, Some(120))?;
    let mut ws = screen.widgets(&mut s, &mut assets, &mut op, &mut rt.host, c)?;
    assert_eq!(screen.layout.name_limit, 20);
    assert!(!screen.bindings.iter().any(|(_, a)| *a == Action::Rename(7)));
    for (slot, added, end, expected) in [
        (8, "  小庭  ", buttons::ENTER, "小庭"),
        (8, "取消", buttons::ESCAPE, "小庭"),
        (8, "　 ", buttons::ENTER, "小庭"),
        (0, "レッスン室", buttons::ENTER, "レッスン室"),
    ] {
        let (r, _) = screen
            .bindings
            .iter()
            .find(|(_, a)| *a == Action::Rename(slot))
            .context("missing rename button")?;
        let mut input = InputFrame {
            pointer_x: (r[0] + r[2] / 2.) as i32,
            pointer_y: (r[1] + r[3] / 2.) as i32,
            buttons_pressed: buttons::MOUSE_LEFT,
            ..Default::default()
        };
        screen.input(
            &mut s,
            &mut ws,
            &mut assets,
            &mut op,
            &mut rt.host,
            c,
            input,
            0,
            0,
        )?;
        input.buttons_pressed = 0;
        screen.input(
            &mut s,
            &mut ws,
            &mut assets,
            &mut op,
            &mut rt.host,
            c,
            input,
            buttons::MOUSE_LEFT,
            20,
        )?;
        assert!(rt.host.text_input.commit(added)?);
        input.buttons_pressed = end;
        screen.input(
            &mut s,
            &mut ws,
            &mut assets,
            &mut op,
            &mut rt.host,
            c,
            input,
            0,
            40,
        )?;
        assert!(screen.editing.is_none());
        assert!(
            ws.iter()
                .any(|w| w.path.ends_with("/InputField Facility Name/Text")
                    && w.label.as_ref().is_some_and(|l| l.text == expected))
        );
    }
    assert!(Catalog::live(&s.retail, 8).is_none());
    operations::start_day(&mut s)?;
    let idx = s
        .retail
        .facility_index(8)
        .context("missing occupied slot after opening")?;
    assert_eq!(upgrade::info(&mut s, idx)?.name, "小庭");
    assert_eq!(upgrade::rooms(&mut s)?[0], "レッスン室");
    Ok(())
}

#[test]
#[ignore = "requires installed day/night scripts, levels and sound archives"]
fn installed_daily_facility_entry_and_return() -> Result<()> {
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
        operations::start_day(s)?;
    }
    for (file, period) in [
        ("daytimemain.ks", daily::Period::Daytime),
        ("nightmain.ks", daily::Period::Night),
    ] {
        let data = files.read(op.storage.as_ref(), file)?;
        let script = encoding_rs::SHIFT_JIS.decode(&data).0;
        let label = script
            .lines()
            .find(|l| l.starts_with("@SceneCall name=SceneDaily "))
            .and_then(|l| {
                l.split_whitespace()
                    .find_map(|arg| arg.strip_prefix("label_facilitymanagement="))
            })
            .context("missing facility menu label")?;
        game.script.load(&files, op.storage.as_ref(), file, label)?;
        game.script.enabled = true;
        game.script.exec();
        game.enter(Scene::Script, &mut op, &mut rt.host, c, 0)?;
        for _ in 0..32 {
            rt.tick_control(250, InputFrame::default())?;
            game.update(&mut rt.host, c, &mut op, InputFrame::default())?;
            if game.scene == Scene::FacilityManagement {
                break;
            }
        }
        assert_eq!(
            game.scene,
            Scene::FacilityManagement,
            "{file} {}",
            game.script.position()
        );
        assert_eq!(game.adv_camera, ([0., 5.5, 11.], 1.4, [180., 3.]));
        let screen = game.construction.as_ref().context("missing room UI")?;
        let (r, _) = screen
            .bindings
            .iter()
            .find(|(_, a)| *a == Action::Ok)
            .context("missing exit control")?;
        let mut input = InputFrame {
            pointer_x: (r[0] + r[2] / 2.) as i32,
            pointer_y: (r[1] + r[3] / 2.) as i32,
            buttons_down: buttons::MOUSE_LEFT,
            buttons_pressed: buttons::MOUSE_LEFT,
            ..Default::default()
        };
        game.update(&mut rt.host, c, &mut op, input)?;
        input.buttons_down = 0;
        input.buttons_pressed = 0;
        game.update(&mut rt.host, c, &mut op, input)?;
        assert_eq!(game.scene, Scene::FacilityManagement);
        rt.tick_control(499, InputFrame::default())?;
        game.update(&mut rt.host, c, &mut op, InputFrame::default())?;
        assert_eq!(game.scene, Scene::FacilityManagement);
        rt.tick_control(1, InputFrame::default())?;
        game.update(&mut rt.host, c, &mut op, InputFrame::default())?;
        assert_eq!(game.scene, Scene::Script);
        for _ in 0..32 {
            rt.tick_control(250, InputFrame::default())?;
            game.update(&mut rt.host, c, &mut op, InputFrame::default())?;
            if game.scene == Scene::Daily(period) {
                break;
            }
        }
        assert_eq!(
            game.scene,
            Scene::Daily(period),
            "{file} {}",
            game.script.position()
        );
    }
    Ok(())
}

#[test]
#[ignore = "requires installed COM3D2 1.0.4 parts and facility data"]
fn installed_facility_wardrobe_controls_and_props() -> Result<()> {
    let (_tmp, mut rt, c, mut op, files, state, mut assets) =
        daily::installed_panel(&["parts", "parts2"])?;
    let script = AdvKag::for_profile(
        &files,
        op.storage.as_ref(),
        crate::tjs::ScriptProfile::Com3d2,
        state,
    )?;
    let mut vm = script.tjs.lock().unwrap();
    operations::start_day(&mut vm.runtime.host_mut().state)?;
    vm.eval("CreateNewMaid('Muku')")?;
    let s = &mut vm.runtime.host_mut().state;
    crate::tjs::retail::finish_profile(s, true)?;
    s.player.money = 500_000;
    let mut screen = Screen::load(s, &mut assets, &mut op, &mut rt.host, c)?;
    screen.catalog.reserve(s, 8, Some(120))?;
    let mut widgets = screen.widgets(s, &mut assets, &mut op, &mut rt.host, c)?;
    for action in [
        Action::ShowTab(Tab::Wardrobe),
        Action::InspectSlot(8),
        Action::ChangeMode(Mode::Default),
        Action::ChangeMode(Mode::Edited),
        Action::EditCostume,
    ] {
        let (r, _) = screen
            .bindings
            .iter()
            .find(|(_, a)| *a == action)
            .with_context(|| format!("missing {action:?}"))?;
        let mut input = InputFrame {
            pointer_x: (r[0] + r[2] / 2.) as i32,
            pointer_y: (r[1] + r[3] / 2.) as i32,
            buttons_pressed: buttons::MOUSE_LEFT,
            ..Default::default()
        };
        screen.input(
            s,
            &mut widgets,
            &mut assets,
            &mut op,
            &mut rt.host,
            c,
            input,
            0,
            0,
        )?;
        input.buttons_pressed = 0;
        screen.input(
            s,
            &mut widgets,
            &mut assets,
            &mut op,
            &mut rt.host,
            c,
            input,
            buttons::MOUSE_LEFT,
            20,
        )?;
        if action == Action::ChangeMode(Mode::Default) {
            assert!(
                widgets
                    .iter()
                    .any(|w| w.path.ends_with("/image thumbnail") && !w.asset.is_empty())
            );
            assert!(
                !screen
                    .bindings
                    .iter()
                    .any(|(_, a)| *a == Action::EditCostume)
            );
        }
    }
    assert!(screen.begin_edit);
    let id = wardrobe::prepare(s, 8)?.context("missing active leader")?;
    let mut editor = stock_editor::StockEditor::default();
    editor.open(s, &[("facility_costume".into(), String::new())].into(), 0)?;
    let mut editor_widgets=assets.scene(&mut op,&mut rt.host,c,"SceneEdit")?;
    editor.load_ui(&mut editor_widgets,&mut assets,&mut op,&mut rt.host,c,&s.retail.stock_models[&id],&s.maids[&id])?;
    for control in ["着衣設定","DeselectAll","SelectAll","skirt"] {
        let diagnostic=editor.diagnostics(s);
        let rect=diagnostic["hits"][control].as_array().with_context(||format!("missing uniform button {control} in {diagnostic}"))?;
        let input=InputFrame {
            pointer_x:(rect[0].as_f64().unwrap()+rect[2].as_f64().unwrap()*0.5) as i32,
            pointer_y:(rect[1].as_f64().unwrap()+rect[3].as_f64().unwrap()*0.5) as i32,
            buttons_pressed:buttons::MOUSE_LEFT,
            ..Default::default()
        };
        editor.update(s,&mut editor_widgets,input,0,&mut op,&mut assets,&mut rt.host,c)?;
        editor.update(s,&mut editor_widgets,InputFrame{buttons_pressed:0,..input},buttons::MOUSE_LEFT,&mut op,&mut assets,&mut rt.host,c)?;
        let parts=wardrobe::enabled(&mut s.retail)?;
        let current=wardrobe::settings(&s.retail,8)?;
        assert_eq!(parts.len(),25);
        let count=parts.iter().filter(|n|current.forced(n)).count();
        assert_eq!(count, match control { "DeselectAll"=>0, "skirt"=>23, _=>25 }, "{control}");
        if control=="skirt" {
            assert!(!current.forced("skirt") && !current.forced("acckamisub"));
            assert!(editor_widgets.iter().any(|w|w.path.ends_with("c_LowerGarment/Item") && !w.asset.is_empty()));
        }
    }
    let prop_index = s.retail.stock_models[&id]
        .prop_names
        .iter()
        .position(|&n| n == "skirt")
        .unwrap();
    let stock_skirt = s.retail.stock_models[&id].props[prop_index].file.clone();
    let shop = crate::tjs::retail::shop::Catalog::open(s, 0)?;
    s.player.money += 1_000_000;
    shop.buy(s, 10)?;
    crate::tjs::retail::edit_wear(s, id, "dress243_skrt_i_.menu")?;
    assert_eq!(
        s.retail.stock_models[&id].props[prop_index].file,
        stock_skirt
    );
    assert_eq!(
        s.retail.stock_models[&id].props[prop_index].temp_file,
        "dress243_skrt_i_.menu"
    );
    wardrobe::finish(s, 8)?;
    assert!(
        s.retail.stock_models[&id].props[prop_index]
            .temp_file
            .is_empty()
    );
    operations::start_day(s)?;
    assert_eq!(Catalog::live(&s.retail, 8), Some(120));
    assert_eq!(wardrobe::settings(&s.retail, 8)?.mode, Mode::Edited);
    assert_eq!(
        wardrobe::settings(&s.retail, 8)?.edits.as_ref().unwrap()["skirt"].as_deref(),
        Some("dress243_skrt_i_.menu")
    );
    assert!(!wardrobe::settings(&s.retail,8)?.forced("skirt"));
    wardrobe::assert_equipped_uniform(&mut vm,id,8)?;
    Ok(())
}
