mod common;

use std::path::Path;

use knapp::config::Root;
use knapp::index::Index;
use knapp::render::Theme;
use knapp::tui::app::{App, Effect, Page};
use knapp::tui::session::Session;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(name)
}

fn root(name: &str, fixture_name: &str) -> Root {
    Root {
        name: Some(name.into()),
        path: fixture(fixture_name),
        send_allow: Vec::new(),
    }
}

fn app(r: &Root) -> App {
    let (index, _) = Index::load(&r.path, &[], None).unwrap();
    App::new(
        index,
        r.name.clone().unwrap_or_default(),
        r.send_allow.clone(),
        Theme { color: false },
    )
}

fn load(r: &Root) -> Result<(App, Option<std::path::PathBuf>), String> {
    Ok((app(r), None))
}

fn press(a: &mut App, c: char) {
    a.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
}

#[test]
fn configured_roots_are_one_to_n() {
    let basic = root("basic", "vault-basic");
    let broken = root("broken", "vault-broken");
    let s = Session::new(
        vec![basic.clone(), broken],
        basic.clone(),
        app(&basic),
        None,
    );
    let keys: Vec<u8> = s.slots.iter().map(|s| s.key).collect();
    assert_eq!(keys, [1, 2]);
    assert_eq!(s.active, 0);
    assert!(s.slots[1].app.is_none(), "loads on first switch");
}

#[test]
fn an_unconfigured_start_root_is_zero() {
    let basic = root("basic", "vault-basic");
    let loose = Root {
        name: None,
        path: fixture("vault-syntax"),
        send_allow: Vec::new(),
    };
    let mut s = Session::new(vec![basic], loose.clone(), app(&loose), None);
    let keys: Vec<u8> = s.slots.iter().map(|s| s.key).collect();
    assert_eq!(keys, [0, 1]);
    assert!(
        s.active().root_label.starts_with("0 "),
        "{}",
        s.active().root_label
    );
}

#[test]
fn switching_keeps_each_roots_place() {
    let basic = root("basic", "vault-basic");
    let broken = root("broken", "vault-broken");
    let mut s = Session::new(
        vec![basic.clone(), broken],
        basic.clone(),
        app(&basic),
        None,
    );
    s.active().open_rel("alpha.md", None, None);
    assert_eq!(s.switch(2, load), Ok(true));
    assert_eq!(s.active().page(), &Page::Summary);
    assert_eq!(s.active().root_label, "2 broken");
    s.active().open_rel("hub.md", None, None);
    assert_eq!(s.switch(1, load), Ok(false));
    assert_eq!(s.active().page(), &Page::Note("alpha.md".into()));
    assert_eq!(s.switch(2, load), Ok(false));
    assert_eq!(s.active().page(), &Page::Note("hub.md".into()));
}

#[test]
fn a_failed_load_or_missing_key_stays_put() {
    let basic = root("basic", "vault-basic");
    let broken = root("broken", "vault-broken");
    let mut s = Session::new(
        vec![basic.clone(), broken],
        basic.clone(),
        app(&basic),
        None,
    );
    assert!(s.switch(2, |_| Err("disk on fire".into())).is_err());
    assert_eq!(s.active, 0);
    assert!(s.switch(7, load).unwrap_err().contains("no root 7"));
    assert!(s.switch(0, load).is_err());
    assert_eq!(s.active, 0);
}

#[test]
fn replies_route_by_slot_and_cell_size_follows() {
    let basic = root("basic", "vault-basic");
    let broken = root("broken", "vault-broken");
    let mut s = Session::new(
        vec![basic.clone(), broken],
        basic.clone(),
        app(&basic),
        None,
    );
    s.active().set_cell_px(Some((8, 16)));
    s.switch(2, load).unwrap();
    assert_eq!(s.active().cell_px, Some((8, 16)));
    // A result for slot 0 lands in slot 0's app, not the active one.
    let other = s.app_mut(0).unwrap();
    other.status = None;
    other.sent(Ok("claude w1:p2".into()));
    assert_eq!(
        s.app_mut(0).unwrap().status.as_deref(),
        Some("sent to claude w1:p2")
    );
    assert_eq!(s.active().status, None);
}

#[test]
fn digits_switch_except_in_lines_and_peek() {
    let basic = root("basic", "vault-basic");
    let mut a = app(&basic);
    press(&mut a, '2');
    assert_eq!(a.take_effects(), [Effect::SwitchRoot(2)]);
    press(&mut a, '/');
    press(&mut a, '2');
    let effects = a.take_effects();
    assert!(
        effects.iter().all(|e| matches!(e, Effect::Search { .. })),
        "{effects:?}"
    );
    assert_eq!(a.query, "2");
    let mut p = app(&basic);
    p.peek = true;
    press(&mut p, '2');
    assert!(p.take_effects().is_empty());
}

#[test]
fn each_root_has_its_own_send_fence() {
    let basic = root("basic", "vault-basic");
    let mut broken = root("broken", "vault-broken");
    broken.send_allow = vec!["".into()];
    let mut s = Session::new(
        vec![basic.clone(), broken],
        basic.clone(),
        app(&basic),
        None,
    );
    s.active().open_rel("alpha.md", None, None);
    press(s.active(), 's');
    assert!(s.active().take_effects().is_empty());
    assert!(s
        .active()
        .status
        .as_deref()
        .unwrap()
        .contains("sending is off"));
    s.switch(2, load).unwrap();
    s.active().open_rel("hub.md", None, None);
    press(s.active(), 's');
    assert_eq!(s.active().take_effects(), [Effect::ListAgents]);
}
