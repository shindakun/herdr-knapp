//! Several roots in one pane: a numbered slot per root, each with its own
//! `App` once shown.

use std::path::PathBuf;

use super::app::App;
use crate::config::Root;

pub struct Slot {
    /// The key that shows it: `0` for an unconfigured start root, `1`..`9`
    /// for configured roots in config order.
    pub key: u8,
    pub root: Root,
    pub app: Option<App>,
    /// This root's cache file, written on quit.
    pub cache: Option<PathBuf>,
}

pub struct Session {
    pub slots: Vec<Slot>,
    pub active: usize,
}

fn same(a: &Root, b: &Root) -> bool {
    crate::config::canonical(&a.path) == crate::config::canonical(&b.path)
}

impl Session {
    /// `start` is the root the pane opened on, already loaded. Configured
    /// roots become slots `1`..; `start` is one of them when it matches, else
    /// slot `0`.
    pub fn new(configured: Vec<Root>, start: Root, app: App, cache: Option<PathBuf>) -> Self {
        let mut slots: Vec<Slot> = Vec::new();
        let at = configured.iter().position(|r| same(r, &start));
        if at.is_none() {
            slots.push(Slot {
                key: 0,
                root: start.clone(),
                app: None,
                cache: None,
            });
        }
        for (i, root) in configured.into_iter().take(9).enumerate() {
            slots.push(Slot {
                key: i as u8 + 1,
                root,
                app: None,
                cache: None,
            });
        }
        let active = match at {
            Some(i) => slots.iter().position(|s| s.key == i as u8 + 1).unwrap_or(0),
            None => 0,
        };
        let mut session = Session { slots, active };
        session.place(active, app, cache);
        session
    }

    fn place(&mut self, index: usize, mut app: App, cache: Option<PathBuf>) {
        if self.slots.len() > 1 {
            let slot = &self.slots[index];
            let name = slot.root.name.clone().unwrap_or_else(|| {
                crate::config::canonical(&slot.root.path)
                    .display()
                    .to_string()
            });
            app.root_label = format!("{} {name}", slot.key);
        }
        self.slots[index].app = Some(app);
        self.slots[index].cache = cache;
    }

    pub fn active(&mut self) -> &mut App {
        self.slots[self.active]
            .app
            .as_mut()
            .expect("the active slot is loaded")
    }

    pub fn app_mut(&mut self, slot: usize) -> Option<&mut App> {
        self.slots.get_mut(slot)?.app.as_mut()
    }

    /// Switches to the slot with `key`, loading it on first use. `Ok(true)`
    /// when a slot was loaded now (its caller starts a watcher), `Ok(false)`
    /// when it was loaded already, `Err` when there is no such slot or it
    /// fails to load; the active slot stays either way.
    pub fn switch(
        &mut self,
        key: u8,
        load: impl FnOnce(&Root) -> Result<(App, Option<PathBuf>), String>,
    ) -> Result<bool, String> {
        let index = self
            .slots
            .iter()
            .position(|s| s.key == key)
            .ok_or_else(|| format!("no root {key}"))?;
        // The cell size follows the terminal, not the root: the new active
        // root takes the current one, loaded or not.
        let px = self.active().cell_px;
        let fresh = self.slots[index].app.is_none();
        if fresh {
            let (app, cache) = load(&self.slots[index].root)?;
            self.place(index, app, cache);
        }
        self.active = index;
        self.active().set_cell_px(px);
        Ok(fresh)
    }

    /// `key  name` for each slot, for the help.
    pub fn listing(&self) -> Vec<String> {
        self.slots
            .iter()
            .map(|s| {
                let name = s
                    .root
                    .name
                    .clone()
                    .unwrap_or_else(|| s.root.path.display().to_string());
                format!("{}  {name}", s.key)
            })
            .collect()
    }
}
