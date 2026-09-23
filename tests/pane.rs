mod common;

use std::collections::BTreeSet;
use std::path::Path;

use common::{rows, temp_copy};
use knapp::index::Index;
use knapp::render::Theme;
use knapp::tui::app::{App, Mode, Page};
use knapp::tui::ui;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;

fn app_for(root: &Path) -> App {
    let (index, _) = Index::load(root, &[], None).expect("load");
    App::new(index, "test".into(), Vec::new(), Theme { color: false })
}

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(name)
}

struct Pane {
    app: App,
    term: Terminal<TestBackend>,
}

impl Pane {
    fn new(root: &Path, w: u16, h: u16) -> Self {
        let mut p = Pane {
            app: app_for(root),
            term: Terminal::new(TestBackend::new(w, h)).unwrap(),
        };
        p.draw();
        p
    }

    fn draw(&mut self) -> Vec<String> {
        self.term.draw(|f| ui::draw(f, &mut self.app)).unwrap();
        rows(self.term.backend().buffer())
    }

    /// Presses each key, drawing after each as the event loop does.
    fn keys(&mut self, keys: &str) -> Vec<String> {
        for c in keys.chars() {
            let code = match c {
                '\n' => KeyCode::Enter,
                '\t' => KeyCode::Tab,
                c => KeyCode::Char(c),
            };
            self.app.key(KeyEvent::new(code, KeyModifiers::NONE));
            self.draw();
        }
        self.draw()
    }

    fn key(&mut self, code: KeyCode) -> Vec<String> {
        self.app.key(KeyEvent::new(code, KeyModifiers::NONE));
        self.draw()
    }

    fn screen(&mut self) -> String {
        self.draw().join("\n")
    }
}

#[test]
fn starts_on_the_summary_with_a_folded_tree() {
    let mut p = Pane::new(&fixture("vault-basic"), 100, 20);
    let screen = p.screen();
    assert!(screen.contains("4 notes, 1 attachments"), "{screen}");
    let rows = p.draw();
    assert!(rows[1].starts_with("▸ dir/"), "{rows:?}");
    assert!(rows[2].starts_with("  alpha.md"), "{rows:?}");
    assert!(!screen.contains("deep.md"));
}

#[test]
fn folders_fold_and_unfold() {
    let mut p = Pane::new(&fixture("vault-basic"), 100, 20);
    let rows = p.keys("\n");
    assert!(rows[1].starts_with("▾ dir/"), "{rows:?}");
    assert!(rows[2].starts_with("    deep.md"), "{rows:?}");
    let rows = p.keys("\n");
    assert!(rows[1].starts_with("▸ dir/"));
}

#[test]
fn follow_a_heading_link_and_go_back() {
    // Short enough that the jump to the heading has to scroll.
    let mut p = Pane::new(&fixture("vault-basic"), 100, 6);
    p.key(KeyCode::End);
    p.keys("\n");
    assert_eq!(p.app.page(), &Page::Note("index.md".into()));

    // n selects the first link in the note: [[alpha]], then the alias, then
    // [[alpha#Second Section]].
    p.keys("lnnn\n");
    assert_eq!(p.app.page(), &Page::Note("alpha.md".into()));
    let rows = p.draw();
    assert_eq!(
        rows[2].trim_start_matches(|c| c != '#'),
        "## Second Section",
        "{rows:?}"
    );

    p.keys("[");
    assert_eq!(p.app.page(), &Page::Note("index.md".into()));
    p.keys("]");
    assert_eq!(p.app.page(), &Page::Note("alpha.md".into()));
}

#[test]
fn unresolved_link_shows_its_references() {
    let mut p = Pane::new(&fixture("vault-broken"), 100, 20);
    // Tree: hub.md is the first file.
    p.keys("\nln\n");
    assert_eq!(p.app.page(), &Page::Unresolved("missing".into()));
    let screen = p.screen();
    assert!(screen.contains("No note named missing"), "{screen}");
    assert!(screen.contains("hub.md:3"), "{screen}");
    assert!(screen.contains("ref.md:3"), "{screen}");
}

#[test]
fn backlinks_open_the_source_at_the_line() {
    let mut p = Pane::new(&fixture("vault-basic"), 100, 20);
    p.keys("j\n"); // alpha.md
    p.keys("\t");
    assert_eq!(p.app.mode, Mode::Backlinks);
    let screen = p.screen();
    assert!(screen.contains("beta.md:8"), "{screen}");
    assert!(screen.contains("index.md:7"), "{screen}");
    p.keys("j\n");
    assert_eq!(p.app.page(), &Page::Note("index.md".into()));
    assert!(p.app.selected_hit.is_some());
}

#[test]
fn forward_mode_lists_links_with_state() {
    let mut p = Pane::new(&fixture("vault-broken"), 100, 20);
    p.keys("\n\t\t");
    assert_eq!(p.app.mode, Mode::Forward);
    let screen = p.screen();
    assert!(screen.contains("✗ [[missing]]  → no note"), "{screen}");
    assert!(screen.contains("[[ref]]  → ref.md"), "{screen}");
}

#[test]
fn narrow_shows_one_panel_at_a_time() {
    let mut p = Pane::new(&fixture("vault-basic"), 60, 20);
    let screen = p.screen();
    assert!(
        screen.contains("dir/") && !screen.contains("4 notes"),
        "{screen}"
    );
    let screen = p.keys("l").join("\n");
    assert!(
        screen.contains("4 notes") && !screen.contains("dir/"),
        "{screen}"
    );
}

#[test]
fn help_opens_and_closes() {
    let mut p = Pane::new(&fixture("vault-basic"), 100, 24);
    assert!(p
        .keys("?")
        .join("\n")
        .contains("back and forward in history"));
    assert!(!p
        .key(KeyCode::Esc)
        .join("\n")
        .contains("back and forward in history"));
}

#[test]
fn refresh_keeps_the_open_note() {
    let root = temp_copy("vault-basic", "pane-refresh");
    let mut p = Pane::new(&root, 100, 20);
    p.key(KeyCode::End);
    p.keys("\n\t"); // index.md, Backlinks
    std::fs::write(root.join("new.md"), "back to [[index]]\n").unwrap();
    p.app.batch(&BTreeSet::from(["new.md".to_string()]));
    let screen = p.screen();
    assert_eq!(p.app.page(), &Page::Note("index.md".into()));
    assert!(screen.contains("new.md:1"), "{screen}");

    std::fs::remove_file(root.join("index.md")).unwrap();
    p.app.batch(&BTreeSet::new());
    assert!(p.screen().contains("deleted: index.md"));
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn quit_keys() {
    let mut p = Pane::new(&fixture("vault-basic"), 100, 20);
    p.keys("q");
    assert!(p.app.quit);
    let mut p = Pane::new(&fixture("vault-basic"), 100, 20);
    p.app
        .key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert!(p.app.quit);
}

use knapp::search::Match;
use knapp::tui::app::Effect;

#[test]
fn o_opens_the_editor_at_the_selected_link_or_top_line() {
    let root = fixture("vault-basic");
    let mut p = Pane::new(&root, 100, 20);
    p.key(KeyCode::End);
    p.keys("\no"); // index.md, nothing selected: the top line
    assert_eq!(
        p.app.take_effects(),
        [Effect::Edit {
            path: std::fs::canonicalize(&root).unwrap().join("index.md"),
            line: 1
        }]
    );
    p.keys("lnnno"); // third link, on line 7
    assert!(matches!(
        p.app.take_effects().as_slice(),
        [Effect::Edit { line: 7, .. }]
    ));

    // An attachment: a status line, no effect.
    p.keys("h");
    p.key(KeyCode::Home);
    p.keys("jjj\no");
    assert!(p.app.take_effects().is_empty());
    assert!(p.screen().contains("o opens notes only"));
}

#[test]
fn y_copies_the_path_and_capital_y_the_wikilink() {
    let root = fixture("vault-basic");
    let mut p = Pane::new(&root, 100, 20);
    p.keys("\nj\n"); // unfold dir/, open dir/deep.md
    p.keys("yY");
    let canonical = std::fs::canonicalize(&root).unwrap();
    assert_eq!(
        p.app.take_effects(),
        [
            Effect::Copy(canonical.join("dir/deep.md").display().to_string()),
            Effect::Copy("[[deep]]".into()),
        ]
    );
}

#[test]
fn capital_y_uses_a_path_when_the_name_is_ambiguous() {
    let mut p = Pane::new(&fixture("vault-ambiguous"), 100, 20);
    p.keys("\nj\n"); // unfold a/, open a/same.md
    p.keys("Y");
    assert_eq!(p.app.take_effects(), [Effect::Copy("[[a/same]]".into())]);
}

#[test]
fn search_query_line_and_stale_results() {
    let mut p = Pane::new(&fixture("vault-basic"), 100, 20);
    p.keys("/alp");
    assert_eq!(p.app.mode, Mode::Search);
    let effects = p.app.take_effects();
    let gens: Vec<u64> = effects
        .iter()
        .map(|e| match e {
            Effect::Search { generation, .. } => *generation,
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(gens, [1, 2, 3]);
    assert!(matches!(&effects[2], Effect::Search { query, .. } if query == "alp"));
    assert!(p.screen().contains("/alp"));

    let m = |rel: &str| Match {
        rel: rel.into(),
        line: 7,
        text: "Links: [[alpha]]".into(),
        ranges: vec![std::ops::Range { start: 9, end: 12 }],
    };
    p.app.results(2, vec![m("index.md")]); // stale
    p.app.results(3, vec![m("index.md"), m("not-indexed.md")]);
    p.keys("\n"); // close the query line
    let screen = p.screen();
    assert!(screen.contains("index.md:7"), "{screen}");
    assert!(!screen.contains("not-indexed"), "{screen}");
    assert_eq!(screen.matches("index.md:7").count(), 1, "{screen}");

    p.keys("\n"); // open the result
    assert_eq!(p.app.page(), &Page::Note("index.md".into()));
}
