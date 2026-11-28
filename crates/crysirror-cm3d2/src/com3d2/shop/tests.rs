// SPDX-License-Identifier: MPL-2.0
use super::*;

#[test]
#[ignore = "requires COM3D2_ROOT standalone installation; detached player state"]
fn installed_shop_controls_and_ownership() -> Result<()> {
    let (_tmp, mut rt, c, mut op, files, mut s, mut assets) =
        daily::installed_panel(&["parts", "parts2"])?;
    let base = assets.scene(&mut op, &mut rt.host, c, "SceneShop")?;
    s.player.current_salon_grade = 1;
    s.player.money = 29_999;
    let mut screen = Screen::load(&mut s, 0, base, &mut assets, &mut op, &mut rt.host, c)?;
    let mut rows = screen.widgets(&s, &mut assets, &op, &mut rt.host, c, &files)?;
    let index = screen
        .catalog
        .categories
        .iter()
        .position(|(m, _)| m == "施設強化素材")
        .context("no enabled material category")?;
    let click = |screen: &mut Screen, want: Action| -> Result<Action> {
        let (r, _) = screen
            .hits
            .iter()
            .find(|(_, a)| *a == want)
            .with_context(|| format!("missing {want:?}"))?;
        let mut input = InputFrame {
            pointer_x: (r[0] + r[2] / 2.) as i32,
            pointer_y: (r[1] + r[3] / 2.) as i32,
            buttons_pressed: buttons::MOUSE_LEFT,
            ..Default::default()
        };
        ensure!(
            (0..1920).contains(&input.pointer_x) && (0..1080).contains(&input.pointer_y),
            "shop hit offscreen {want:?} at {r:?}"
        );
        assert!(screen.action(&input, 0).is_none());
        input.buttons_pressed = 0;
        screen
            .action(&input, buttons::MOUSE_LEFT)
            .context("missing shop release")
    };
    ensure!(
        rows.iter().any(|w| w.path.ends_with("/WorkingFunds/Value")
            && w.label.as_ref().is_some_and(|l| l.text == "29,999")),
        "money label"
    );
    let action = click(&mut screen, Action::Main(index))?;
    screen.select(action);
    screen.widgets(&s, &mut assets, &op, &mut rt.host, c, &files)?;
    let sub = screen
        .catalog
        .categories
        .iter()
        .position(|cat| cat.0 == "施設強化素材" && cat.1 == "資材セット")
        .context("missing material set tab")?;
    let a = click(&mut screen, Action::Sub(sub))?;
    screen.select(a);
    rows = screen.widgets(&s, &mut assets, &op, &mut rt.host, c, &files)?;
    assert!(
        rows.iter()
            .any(|w| w.label.as_ref().is_some_and(|l| l.text == "下級設備資材"))
    );
    let (r, _, _, _) = screen
        .icons
        .iter()
        .find(|(_, id, ..)| *id == 10000)
        .context("no material info collider")?;
    let hover = InputFrame {
        pointer_x: (r[0] + r[2] / 2.) as i32,
        pointer_y: (r[1] + r[3] / 2.) as i32,
        ..Default::default()
    };
    screen.show_info(&mut rows, &hover);
    assert!(rows.iter().any(|w| w.path.ends_with("/Base/Info")
        && w.label.as_ref().is_some_and(|l| l.text.contains("ネジ"))));
    assert!(screen.catalog.buy(&mut s, 10000).is_err());
    assert!(!screen.hits.iter().any(|(_, a)| *a == Action::Buy(10000)));
    assert_eq!(s.player.money, 29_999);
    s.player.money = 30_000;
    screen.widgets(&s, &mut assets, &op, &mut rt.host, c, &files)?;
    assert_eq!(click(&mut screen, Action::Buy(10000))?, Action::Buy(10000));
    screen.catalog.buy(&mut s, 10000)?;
    assert_eq!((s.player.money, s.player.shop_use_money), (0, 30_000));
    assert!(s.retail.materials.contains(&10));
    assert!(screen.catalog.owns(&s, 10000));
    assert_eq!(
        s.player.shop_lineup.get(&10000),
        Some(&crate::params::SHOP_NEW)
    );
    assert!(screen.catalog.buy(&mut s, 10000).is_err());
    screen.widgets(&s, &mut assets, &op, &mut rt.host, c, &files)?;
    assert!(!screen.hits.iter().any(|(_, a)| *a == Action::Buy(10000)));
    assert_eq!(click(&mut screen, Action::Exit)?, Action::Exit);
    s.player.money = 5_000_000;
    let bundle = screen.catalog.price(&s, 5000)?;
    let skirt = screen.catalog.price(&s, 10)?;
    screen.catalog.buy(&mut s, 10)?;
    assert_eq!(screen.catalog.price(&s, 5000)?, bundle - skirt);
    assert!(!s.retail.editor_owns("_i_mywear243.menu"));
    screen.catalog.buy(&mut s, 5000)?;
    assert!(s.retail.editor_owns("_i_mywear243.menu"));
    assert!(s.retail.editor_owns("dress243_skrt_i_.menu"));
    assert_eq!(s.player.money, 5_000_000 - bundle);
    Ok(())
}

#[test]
#[ignore = "installed 1.0.4 data; set COM3D2_MOTION_ARCHIVE for a bz2-wrapped installation"]
fn installed_shop_day_and_night_callers() -> Result<()> {
    let packs = ["parts", "parts2", "sound", "sound2", "motion2"];
    let (tmp, mut rt, c, mut op, _, _, _) = daily::installed_panel(&packs)?;
    super::super::installed_menu::mount_motion(&mut rt,c,&tmp)?;
    let cfg = Config {
        data_dir: op.dir.clone(),
        archives: ["csv", "system", "system2", "script", "motion"]
            .into_iter()
            .chain(packs)
            .map(str::to_owned)
            .collect(),
    };
    let mut vm = Vm {
        linked_cm3d2: None,
        client: c,
        config: cfg,
        game: None,
    };
    vm.advance(&mut rt.host, InputFrame::default())?;
    let game = vm.game.as_mut().unwrap();
    {
        let mut vm = game.script.tjs.lock().unwrap();
        let s = &mut vm.runtime.host_mut().state;
        s.player.schedule_slot.resize_with(40, Default::default);
        crate::tjs::retail::operations::start_day(s)?;
        vm.eval("CreateNewMaid('Muku')")?;
        let s = &mut vm.runtime.host_mut().state;
        crate::tjs::retail::finish_profile(s, true)?;
        s.player.money = 30_000;
    }
    let mut now = 1000;
    for (file, label) in [("DaytimeMain", "*昼メニュー"), ("NightMain", "*夜メニュー")] {
        game.script
            .load(&game.files, op.storage.as_ref(), file, "*ショップ")?;
        game.script.exec();
        let mut entered = false;
        for _ in 0..40 {
            now += 1000;
            let step = game
                .script
                .update(now, true, false, &game.files, op.storage.as_ref())?;
            if let KagStep::SceneCall(name) = step {
                if name == "SceneADV" {
                    game.script.enabled = true;
                    game.script.exec();
                    continue;
                }
                ensure!(name == "SceneShop", "unexpected destination {name}");
                ensure!(
                    game.script
                        .tag_backup
                        .get("label")
                        .is_some_and(|v| v == label),
                    "wrong shop return"
                );
                game.enter(Scene::Shop, &mut op, &mut rt.host, c, now)?;
                entered = true;
                break;
            }
        }
        ensure!(entered, "{file} did not call SceneShop");
        game.update(&mut rt.host, c, &mut op, InputFrame::default())?;
        assert!(game.shop.initial_camera.is_some());
        if file == "DaytimeMain" {
            let cat = game
                .shop
                .catalog
                .categories
                .iter()
                .position(|(m, _)| m == "施設強化素材")
                .unwrap();
            let sub = game
                .shop
                .catalog
                .categories
                .iter()
                .position(|(m, s)| m == "施設強化素材" && s == "資材セット")
                .unwrap();
            let garments = game
                .shop
                .catalog
                .categories
                .iter()
                .position(|(m, _)| m == "衣装")
                .unwrap();
            let skirts = game
                .shop
                .catalog
                .categories
                .iter()
                .position(|(m, s)| m == "衣装" && s == "ボトムス")
                .unwrap();
            let before = game
                .actors
                .actor(false, 0)
                .unwrap()
                .file("skirt")
                .unwrap_or("")
                .to_owned();
            for (action, button) in [
                (Some(Action::Main(garments)), ""),
                (Some(Action::Sub(skirts)), ""),
                (Some(Action::Icon(10)), ""),
                (Some(Action::TryOn), ""),
                (Some(Action::Restore), ""),
                (Some(Action::Main(cat)), ""),
                (Some(Action::Sub(sub)), ""),
                (Some(Action::Buy(10000)), ""),
                (None, "Cancel"),
                (Some(Action::Buy(10000)), ""),
                (None, "Ok"),
                (None, "Ok"),
            ] {
                let rect = if let Some(action) = action {
                    game.shop
                        .hits
                        .iter()
                        .find(|(_, a)| *a == action)
                        .map(|v| v.0)
                        .with_context(|| format!("missing {action:?}"))?
                } else {
                    let start = game
                        .shop
                        .dialog
                        .as_ref()
                        .context("missing purchase dialog")?
                        .first;
                    game.widgets[start..]
                        .iter()
                        .find(|w| w.path.ends_with(&format!("/Base/{button}")))
                        .and_then(|w| w.collider)
                        .context("missing dialog collider")?
                };
                let mut input = InputFrame {
                    pointer_x: (rect[0] + rect[2] / 2.) as i32,
                    pointer_y: (rect[1] + rect[3] / 2.) as i32,
                    buttons_pressed: buttons::MOUSE_LEFT,
                    ..Default::default()
                };
                game.update_shop(&mut op, &mut rt.host, c, input, 0, now)?;
                input.buttons_pressed = 0;
                now += 20;
                game.update_shop(&mut op, &mut rt.host, c, input, buttons::MOUSE_LEFT, now)?;
                now += 400;
                game.update_shop(&mut op, &mut rt.host, c, input, 0, now)?;
                if action == Some(Action::TryOn) {
                    assert_eq!(
                        game.actors.actor(false, 0).unwrap().file("skirt"),
                        Some("dress243_skrt_i_.menu")
                    );
                } else if action == Some(Action::Restore) {
                    assert_eq!(
                        game.actors
                            .actor(false, 0)
                            .unwrap()
                            .file("skirt")
                            .unwrap_or(""),
                        before
                    );
                }
                if button == "Cancel" {
                    let vm = game.script.tjs.lock().unwrap();
                    let s = &vm.runtime.host().state;
                    assert_eq!(s.player.money, 30_000);
                    assert!(!s.retail.materials.contains(&10));
                }
            }
        }
        let r = game
            .shop
            .hits
            .iter()
            .find(|(_, a)| *a == Action::Exit)
            .unwrap()
            .0;
        let mut input = InputFrame {
            pointer_x: (r[0] + r[2] / 2.) as i32,
            pointer_y: (r[1] + r[3] / 2.) as i32,
            buttons_pressed: buttons::MOUSE_LEFT,
            ..Default::default()
        };
        game.update_shop(&mut op, &mut rt.host, c, input, 0, now)?;
        input.buttons_pressed = 0;
        game.update_shop(
            &mut op,
            &mut rt.host,
            c,
            input,
            buttons::MOUSE_LEFT,
            now + 20,
        )?;
        assert_eq!(game.scene, Scene::Script);
        let mut returned = false;
        for _ in 0..40 {
            now += 1000;
            if let KagStep::SceneCall(name) =
                game.script
                    .update(now, true, false, &game.files, op.storage.as_ref())?
            {
                if name == "SceneADV" {
                    game.script.enabled = true;
                    game.script.exec();
                    continue;
                }
                let period = if file == "DaytimeMain" {
                    "Daytime"
                } else {
                    "Night"
                };
                ensure!(
                    name == "SceneDaily"
                        && game
                            .script
                            .tag_backup
                            .get("type")
                            .is_some_and(|v| v == period),
                    "shop returned to {name} at {}",
                    game.script.position()
                );
                returned = true;
                break;
            }
        }
        ensure!(returned, "shop did not resume its daily menu");
        let vm = game.script.tjs.lock().unwrap();
        let s = &vm.runtime.host().state;
        assert_eq!((s.player.money, s.player.shop_use_money), (0, 30_000));
        assert!(s.retail.materials.contains(&10));
    }
    Ok(())
}
