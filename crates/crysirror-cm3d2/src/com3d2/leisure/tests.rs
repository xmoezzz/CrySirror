// SPDX-License-Identifier: MPL-2.0
use super::super::installed_menu::Menu;
use super::*;
fn click(m: &mut Menu, end: &str) -> Result<()> {
    let r = m
        .game
        .widgets
        .iter()
        .rev()
        .find(|w| w.path.ends_with(end) && w.collider.is_some())
        .and_then(|w| w.collider)
        .with_context(|| format!("missing hit target {end} in {:?}", m.game.scene))?;
    m.click(r)
}
fn open_daily_button(
    m: &mut Menu,
    night: bool,
    button: daily::Button,
    destination: Destination,
) -> Result<()> {
    let period = if night {
        daily::Period::Night
    } else {
        daily::Period::Daytime
    };
    m.open(
        night,
        if night {
            "*夜メニュー"
        } else {
            "*昼メニュー"
        },
        Scene::Daily(period),
    )?;
    if !m.game.daily.pages.is_empty() {
        click(m, "/Button OK")?;
        m.tick(501)?;
    }
    let rect = m
        .game
        .widgets
        .iter()
        .find(|w| w.button.and_then(|id| m.game.assets.daily_buttons.get(&id)) == Some(&button))
        .and_then(|w| w.collider)
        .context("daily button not interactable")?;
    m.click(rect)?;
    m.wait(Scene::Leisure(destination))?;
    m.tick(500)
}
#[test]
#[ignore = "installed standalone 1.0.4 scene and archives"]
fn installed_photo_menu() -> Result<()> {
    let mut m = Menu::new()?;
    open_daily_button(&mut m, false, daily::Button::PhotoMode, Photography)?;
    assert!(m.game.club_meshes > 0);
    click(&mut m, "/UIOFF")?;
    assert!(
        m.game
            .widgets
            .iter()
            .all(|w| w.path.contains("/UION") || w.path.contains("/ScreenShot"))
    );
    click(&mut m, "/UION")?;
    click(&mut m, "photo/background/1")?;
    assert_eq!(
        m.game
            .script
            .tjs
            .lock()
            .unwrap()
            .runtime
            .host()
            .state
            .bg_name,
        "EmpireClub_Rotary_Night"
    );
    click(&mut m, "WindowVisibleBtnsLine2/End")?;
    click(&mut m, "/Base/Cancel")?;
    m.tick(301)?;
    assert_eq!(m.game.scene, Scene::Leisure(Photography));
    click(&mut m, "WindowVisibleBtnsLine2/End")?;
    click(&mut m, "/Base/Ok")?;
    m.wait(Scene::Daily(daily::Period::Daytime))
}
#[test]
#[ignore = "installed standalone 1.0.4 scene and archives"]
fn installed_creative_room() -> Result<()> {
    let mut m = Menu::new()?;
    open_daily_button(&mut m, true, daily::Button::MyRoomCustom, CreativeRoom)?;
    let initial = m.game.club_meshes;
    assert_eq!(initial, 10);
    click(&mut m, "/Slider SizeX/HitArea")?;
    assert!(m.game.club_meshes > initial);
    let edited = m.game.club_meshes;
    click(&mut m, "/Button Save")?;
    click(&mut m, "/Element/Button Element")?;
    click(&mut m, "/Base/Ok")?;
    click(&mut m, "/Button Save")?;
    click(&mut m, "/Slider SizeZ/HitArea")?;
    assert_ne!(edited, m.game.club_meshes);
    click(&mut m, "/Button Load")?;
    click(&mut m, "/Element/Button Element")?;
    click(&mut m, "/Base/Ok")?;
    assert_eq!(edited, m.game.club_meshes);
    click(&mut m, "/Button Quit")?;
    click(&mut m, "/Base/Ok")?;
    m.wait(Scene::Daily(daily::Period::Night))
}
#[test]
#[ignore = "installed standalone 1.0.4 scene and archives"]
fn installed_examination() -> Result<()> {
    let mut m = Menu::with_motion(true)?;
    open_daily_button(&mut m, false, daily::Button::CompetitiveShow, Examination)?;
    m.tick(6400)?;
    assert!(m.game.widgets.is_empty());
    m.tick(3500)?;
    let key = if m.game.leisure.dialog.is_some() {
        "/Base/Ok"
    } else {
        "/Viewer/Ok"
    };
    click(&mut m, key)?;
    assert_eq!(m.game.scene, Scene::Script);
    assert!(
        m.game
            .script
            .position()
            .to_lowercase()
            .contains("daytimemain")
    );
    assert_eq!(
        m.game
            .script
            .tjs
            .lock()
            .unwrap()
            .runtime
            .host()
            .state
            .tmp_flags
            .get("品評会_演出省略"),
        Some(&0)
    );
    // The original caller plays the entrant's result dialogue before reopening the menu.
    for _ in 0..160 {
        if m.game.scene == Scene::Daily(daily::Period::Daytime) {
            return Ok(());
        }
        if m.game.script.hitret_since().is_some() {
            m.click([900., 800., 8., 8.])?;
        }
        m.tick(250)?;
    }
    bail!(
        "examination return stalled at {} ({})",
        m.game.script.position(),
        m.game.script.wait_state()
    )
}
#[test]
#[ignore = "installed standalone 1.0.4 scene and archives"]
fn installed_casino() -> Result<()> {
    let mut m = Menu::with_motion(true)?;
    {
        let mut vm = m.game.script.tjs.lock().unwrap();
        let s = &mut vm.runtime.host_mut().state;
        let cat = crate::tjs::retail::construction::Catalog::load(&mut s.retail)?;
        s.player.money = cat.get(170)?.cost + 310_000;
        cat.reserve(s, 8, Some(170))?;
        crate::tjs::retail::operations::start_day(s)?;
        assert_eq!(
            crate::tjs::retail::construction::Catalog::live(&s.retail, 8),
            Some(170)
        );
    }
    open_daily_button(&mut m, true, daily::Button::Casino, Casino)?;
    click(&mut m, "/CasinoMenu/SelectDealer")?;
    let selected = m
        .game
        .widgets
        .iter()
        .find(|w| w.path.starts_with("dealer/"))
        .and_then(|w| w.collider)
        .context("dealer roster")?;
    m.click(selected)?;
    click(&mut m, "/Ok_Button")?;
    click(&mut m, "/CasinoMenu/BlackJack")?;
    assert!(m.game.leisure.dialog.is_some());
    click(&mut m, "/Base/Ok")?;
    click(&mut m, "/ButtonKankin")?;
    click(&mut m, "/ExChangeUI/Max")?;
    click(&mut m, "/ExChangeUI/Cancel")?;
    {
        let vm = m.game.script.tjs.lock().unwrap();
        assert_eq!(vm.runtime.host().state.retail.casino_coins, 0);
    }
    click(&mut m, "/ButtonKankin")?;
    click(&mut m, "/ExChangeUI/Max")?;
    click(&mut m, "/ExChangeUI/OK")?;
    {
        let vm = m.game.script.tjs.lock().unwrap();
        let s = &vm.runtime.host().state;
        assert_eq!((s.player.money, s.retail.casino_coins), (0, 310));
    }
    click(&mut m, "/CasinoMenu/Cancel_Button")?;
    m.wait(Scene::Daily(daily::Period::Night))
}

#[test]
#[ignore = "installed standalone 1.0.4 casino shop"]
fn installed_casino_shop_purchase_and_return() -> Result<()> {
    let mut m = Menu::with_motion(true)?;
    {
        let mut vm = m.game.script.tjs.lock().unwrap();
        let s = &mut vm.runtime.host_mut().state;
        let cat = crate::tjs::retail::construction::Catalog::load(&mut s.retail)?;
        s.player.money = cat.get(170)?.cost;
        cat.reserve(s, 8, Some(170))?;
        crate::tjs::retail::operations::start_day(s)?;
        s.retail.casino_coins = 900_000;
    }
    open_daily_button(&mut m, true, daily::Button::Casino, Casino)?;
    click(&mut m, "/CasinoMenu/ItemExchange")?;
    m.wait(Scene::Leisure(CasinoShop))?;
    m.tick(500)?;
    click(&mut m, "casino-shop/category/1")?;
    let key = m.game.widgets.iter().find(|w| {
        w.path.starts_with("casino-shop/item/") && w.path.ends_with("/BuyButton") && w.collider.is_some()
    }).context("casino material purchase button")?.path.clone();
    click(&mut m, &key)?;
    click(&mut m, "/Base/Cancel")?;
    m.tick(301)?;
    {
        let vm = m.game.script.tjs.lock().unwrap();
        assert_eq!(vm.runtime.host().state.retail.casino_coins, 900_000);
    }
    click(&mut m, &key)?;
    click(&mut m, "/Base/Ok")?;
    click(&mut m, "/Base/Ok")?;
    {
        let vm = m.game.script.tjs.lock().unwrap();
        let s = &vm.runtime.host().state.retail;
        assert!(s.casino_coins < 900_000);
        assert!(s.casino_sold_out.contains(&1000));
        assert!(s.materials.contains(&30));
    }
    assert!(!m.game.widgets.iter().any(|w| w.path == key));
    click(&mut m, "UI Root/Parent/OK")?;
    m.wait(Scene::Leisure(Casino))
}

#[test]
#[ignore = "installed standalone 1.0.4 blackjack"]
fn installed_blackjack_bet_settle_and_return() -> Result<()> {
    let mut m = Menu::with_motion(true)?;
    {
        let mut vm = m.game.script.tjs.lock().unwrap();
        let s = &mut vm.runtime.host_mut().state;
        let cat = crate::tjs::retail::construction::Catalog::load(&mut s.retail)?;
        s.player.money = cat.get(170)?.cost;
        cat.reserve(s, 8, Some(170))?;
        crate::tjs::retail::operations::start_day(s)?;
        s.retail.casino_coins = 10_000;
        s.player.generic_flag.insert("BJ最終プレイ日".into(), s.player.days);
    }
    open_daily_button(&mut m, true, daily::Button::Casino, Casino)?;
    click(&mut m, "/CasinoMenu/BlackJack")?;
    m.wait(Scene::Leisure(Blackjack))?;
    m.tick(500)?;
    let actor = m.game.actors.actor(false, 0).context("blackjack dealer")?;
    assert!(actor.visible);
    assert!(!actor.anims.is_empty());
    let stage = m.rt.render_pbr()?.context("blackjack world")?;
    assert!(stage.instances.iter().any(|i| (i.key as u32) >= 0x1000));
    click(&mut m, "/Frame/StartButton")?;
    click(&mut m, "/BetSetUI/OK")?;
    if m.game.widgets.iter().any(|w| w.path == "/CallUIPanel/Stand" && w.collider.is_some()) {
        click(&mut m, "/CallUIPanel/Stand")?;
    }
    assert!(m.game.widgets.iter().any(|w| w.path == "/ResultBG/GetMoneyText"));
    click(&mut m, "/ResultBG/Next")?;
    click(&mut m, "/Frame/EndButton")?;
    click(&mut m, "/Base/Ok")?;
    m.wait(Scene::Leisure(Casino))
}
