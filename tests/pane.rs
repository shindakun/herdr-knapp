mod common;

use std::collections::BTreeSet;
use std::path::Path;

use common::{rows, temp_copy};
use knapp::index::Index;
use knapp::render::Theme;
use knapp::tui::app::{App, Mode, Page};
use knapp::tui::ui;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
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

    /// A left click at zero-based `col`, `row`.
    fn click(&mut self, col: u16, row: u16) -> String {
        self.app.mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        });
        self.screen()
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
fn narrow_opening_a_note_shows_it() {
    let mut p = Pane::new(&fixture("vault-basic"), 60, 20);
    let screen = p.keys("j\n").join("\n");
    assert!(
        screen.contains("# Alpha") && !screen.contains("dir/"),
        "{screen}"
    );

    let mut p = Pane::new(&fixture("vault-basic"), 60, 20);
    let screen = p.click(2, 2);
    assert!(
        screen.contains("# Alpha") && !screen.contains("dir/"),
        "{screen}"
    );
}

#[test]
fn clicks_do_nothing_under_help() {
    let mut p = Pane::new(&fixture("vault-basic"), 100, 24);
    p.keys("?");
    p.click(2, 2);
    assert_eq!(p.app.page(), &Page::Summary);
    p.key(KeyCode::Esc);
    p.click(2, 2);
    assert_eq!(p.app.page(), &Page::Note("alpha.md".into()));
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

use knapp::tui::app::age;

#[test]
fn age_units() {
    let cases = [
        (0, "0s"),
        (59, "59s"),
        (60, "1m"),
        (3_599, "59m"),
        (3_600, "1h"),
        (86_399, "23h"),
        (86_400, "1d"),
        (7 * 86_400, "1w"),
        (29 * 86_400, "4w"),
        (30 * 86_400, "1mo"),
        (364 * 86_400, "12mo"),
        (365 * 86_400, "1y"),
    ];
    for (secs, want) in cases {
        assert_eq!(age(secs), want, "{secs}");
    }
}

#[test]
fn tab_visits_every_mode_in_order() {
    let mut p = Pane::new(&fixture("vault-basic"), 120, 20);
    let mut seen = vec![p.app.mode];
    for _ in 0..8 {
        p.keys("\t");
        seen.push(p.app.mode);
    }
    assert_eq!(
        seen,
        [
            Mode::Tree,
            Mode::Backlinks,
            Mode::Forward,
            Mode::Tags,
            Mode::Unresolved,
            Mode::Orphans,
            Mode::Recent,
            Mode::Search,
            Mode::Tree
        ]
    );
}

#[test]
fn tags_unfold_and_open_a_note() {
    let mut p = Pane::new(&fixture("vault-basic"), 100, 20);
    p.keys("\t\t\t");
    assert_eq!(p.app.mode, Mode::Tags);
    let rows = p.draw();
    assert!(rows[1].starts_with("▸ #heading-tag  1"), "{rows:?}");
    // meta is the fourth tag: heading-tag, home, inline-tag, meta.
    p.keys("jjj\n");
    let rows = p.draw();
    assert!(rows[4].starts_with("▾ #meta  1"), "{rows:?}");
    assert!(rows[5].starts_with("  ▸ #index  1"), "{rows:?}");
    p.keys("j\nj\n"); // unfold index, open index.md under it
    assert_eq!(p.app.page(), &Page::Note("index.md".into()));
}

#[test]
fn ambiguous_target_page_lists_candidates_and_sources() {
    let mut p = Pane::new(&fixture("vault-ambiguous"), 100, 20);
    p.keys("\t\t\t\t");
    assert_eq!(p.app.mode, Mode::Unresolved);
    let screen = p.screen();
    assert!(screen.contains("?   3  same"), "{screen}");
    p.keys("\n");
    assert_eq!(p.app.page(), &Page::Ambiguous("same".into()));
    let screen = p.screen();
    assert!(screen.contains("same matches 3 files"), "{screen}");
    // Obsidian's pick depends on the linking note's folder.
    assert!(screen.contains("root.md:3 → a/same.md"), "{screen}");
    assert!(screen.contains("b/linker.md:3 → b/same.md"), "{screen}");
    // The candidates are sorted by path; follow the first.
    p.keys("ln\n");
    assert_eq!(p.app.page(), &Page::Note("a/same.md".into()));
}

#[test]
fn orphans_and_recent() {
    let root = temp_copy("vault-broken", "pane-recent");
    let mut p = Pane::new(&root, 100, 20);
    p.keys("\t\t\t\t\t");
    assert_eq!(p.app.mode, Mode::Orphans);
    let rows = p.draw();
    assert!(
        rows[1].starts_with("orphan.md") && rows[2].starts_with("self.md"),
        "{rows:?}"
    );

    // temp_copy ages every file a minute; touch one and it leads Recent.
    std::fs::write(root.join("loop-b.md"), "# Loop B\n\n[[loop-c]] edited\n").unwrap();
    p.app.batch(&BTreeSet::from(["loop-b.md".to_string()]));
    p.keys("\t");
    assert_eq!(p.app.mode, Mode::Recent);
    let rows = p.draw();
    assert!(
        rows[1].trim_start().starts_with("0s  loop-b.md"),
        "{rows:?}"
    );
    assert!(rows[2].trim_start().starts_with("1m  "), "{rows:?}");
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn narrow_header_keeps_the_mode_visible() {
    let mut p = Pane::new(&fixture("vault-basic"), 100, 10);
    assert!(p.draw()[0].contains("Tree Backlinks Forward Tags"));
    let mut p = Pane::new(&fixture("vault-basic"), 40, 10);
    p.keys("\t\t\t");
    let header = p.draw()[0].clone();
    assert!(header.contains("◂ Tags ▸"), "{header}");
    assert!(header.ends_with("│ send off"), "{header}");
    assert!(
        unicode_width::UnicodeWidthStr::width(header.as_str()) <= 40,
        "{header}"
    );

    // With send_allow, the prefixes stay in a 36-column header.
    let mut p = allowed_pane(&fixture("vault-basic"));
    p.term = Terminal::new(TestBackend::new(36, 10)).unwrap();
    p.app.send_allow = vec!["notes/".into()];
    let header = p.draw()[0].clone();
    assert!(header.contains("│ send: notes/"), "{header}");
}

#[test]
fn the_open_note_shows_a_prose_edit() {
    let root = temp_copy("vault-basic", "pane-prose");
    let mut p = Pane::new(&root, 100, 20);
    p.keys("j\n"); // alpha.md
    assert!(!p.screen().contains("More prose"));
    std::fs::write(
        root.join("alpha.md"),
        "# Alpha\n\nFirst paragraph. ^para1\n\n## Second Section\n\nBack to [[index]]. More prose.\n",
    )
    .unwrap();
    p.app.batch(&BTreeSet::from(["alpha.md".to_string()]));
    assert!(p.screen().contains("More prose"), "{}", p.screen());
    std::fs::remove_dir_all(root).ok();
}

fn agent(pane: &str, name: &str) -> knapp::herdr::Agent {
    serde_json::from_value(serde_json::json!({
        "pane_id": pane, "workspace_id": "w1", "agent": name, "agent_status": "idle"
    }))
    .unwrap()
}

#[test]
fn send_is_refused_without_send_allow() {
    let mut p = Pane::new(&fixture("vault-basic"), 100, 20);
    p.keys("j\ns"); // alpha.md
    assert!(p.app.take_effects().is_empty());
    let screen = p.screen();
    assert!(screen.contains("not sent: sending is off"), "{screen}");
}

fn allowed_pane(root: &Path) -> Pane {
    let (index, _) = Index::load(root, &[], None).unwrap();
    let app = App::new(
        index,
        "test".into(),
        vec!["".into()],
        Theme { color: false },
    );
    let mut p = Pane {
        app,
        term: Terminal::new(TestBackend::new(100, 20)).unwrap(),
    };
    p.draw();
    p
}

#[test]
fn send_line_with_one_agent_then_enter() {
    let mut p = allowed_pane(&fixture("vault-basic"));
    p.keys("j\ns"); // alpha.md
    assert_eq!(p.app.take_effects(), [Effect::ListAgents]);
    p.app.agents(Ok(vec![agent("w1:p2", "claude")]), None);
    let screen = p.screen();
    assert!(screen.contains("to claude w1:p2 · 1 note · "), "{screen}");

    p.keys("what next?\n");
    let effects = p.app.take_effects();
    let [Effect::Send { pane, agent, text }] = effects.as_slice() else {
        panic!("{effects:?}");
    };
    assert_eq!((pane.as_str(), agent.as_str()), ("w1:p2", "claude w1:p2"));
    assert!(
        text.starts_with("what next?\n\nThe notes below are reference data"),
        "{text}"
    );
    assert!(text.contains(" path=\"alpha.md\">\n# Alpha"), "{text}");
    p.app.sent(Ok("claude w1:p2".into()));
    assert!(p.screen().contains("sent to claude w1:p2"));
}

#[test]
fn esc_cancels_and_capital_s_adds_backlinks() {
    let mut p = allowed_pane(&fixture("vault-basic"));
    p.keys("j\ns");
    p.app.take_effects();
    p.app.agents(Ok(vec![agent("w1:p2", "claude")]), None);
    p.key(KeyCode::Esc);
    assert!(p.app.take_effects().is_empty());
    assert!(p.screen().contains("not sent"));

    p.keys("S");
    p.app.take_effects();
    p.app.agents(Ok(vec![agent("w1:p2", "claude")]), None);
    assert!(p.screen().contains("· 3 notes ·"), "{}", p.screen());
    p.keys("\n");
    let Some(Effect::Send { text, .. }) = p.app.take_effects().pop() else {
        panic!("no send");
    };
    let paths: Vec<&str> = text
        .match_indices(" path=\"")
        .map(|(i, _)| {
            let rest = &text[i + 7..];
            &rest[..rest.find('"').unwrap()]
        })
        .collect();
    assert_eq!(paths, ["alpha.md", "beta.md", "index.md"]);
}

#[test]
fn several_agents_open_a_picker_at_the_last_one() {
    let mut p = allowed_pane(&fixture("vault-basic"));
    p.keys("j\ns");
    p.app.take_effects();
    p.app.agents(
        Ok(vec![agent("w1:p2", "claude"), agent("w1:p5", "codex")]),
        Some("w1:p5".into()),
    );
    let screen = p.screen();
    assert!(
        screen.contains("send to") && screen.contains("codex w1:p5"),
        "{screen}"
    );
    assert_eq!(p.app.picker.as_ref().unwrap().selected, 1);
    p.keys("k\n");
    assert!(p.screen().contains("to claude w1:p2"), "{}", p.screen());

    p.key(KeyCode::Esc);
    p.keys("s");
    p.app.take_effects();
    p.app.agents(Ok(Vec::new()), None);
    assert!(p.screen().contains("no agent in this workspace"));
    p.keys("s");
    p.app.take_effects();
    p.app.agents(Err("sending needs herdr".into()), None);
    assert!(p.screen().contains("not sent: sending needs herdr"));
}

#[test]
fn graph_page_without_graphics_is_the_tree() {
    let mut p = Pane::new(&fixture("vault-basic"), 100, 20);
    p.keys("j\ng"); // alpha.md, then its graph
    assert_eq!(p.app.page(), &Page::Graph("alpha.md".into()));
    let screen = p.screen();
    assert!(screen.contains("graph: alpha.md"), "{screen}");
    assert!(screen.contains("<- beta.md"), "{screen}");
    assert!(screen.contains("<-> index.md"), "{screen}");
    assert!(p.app.layers.is_empty());
    // Names are links.
    p.keys("ln\n");
    assert_eq!(p.app.page(), &Page::Note("alpha.md".into()));
}

#[test]
fn graph_page_with_graphics_has_a_canvas_layer() {
    let mut p = Pane::new(&fixture("vault-basic"), 100, 30);
    p.app.cell_px = Some((8, 16));
    p.keys("j\ng");
    p.draw();
    assert_eq!(p.app.layers.len(), 1);
    let layer = &p.app.layers[0];
    assert_eq!(layer.name, "graph");
    let knapp::tui::app::LayerContent::Canvas(spec) = &layer.content else {
        panic!("not the canvas");
    };
    assert_eq!(spec.dots.len(), 5);
    // The canvas sits under the detail's first content row.
    assert_eq!((layer.at.row, layer.at.cols), (2, spec.cols));
    let screen = p.screen();
    for name in ["alpha", "beta", "index", "deep", "img.png"] {
        assert!(screen.contains(name), "{name} missing:\n{screen}");
    }
    // Help hides the image; closing it brings it back.
    p.keys("?");
    assert!(p.app.layers.is_empty());
    p.key(KeyCode::Esc);
    assert_eq!(p.app.layers.len(), 1);

    // In a short pane the page scrolls, and the image moves with it.
    let mut p = Pane::new(&fixture("vault-basic"), 100, 16);
    p.app.cell_px = Some((8, 16));
    p.keys(
        "j
gl",
    );
    assert_eq!(p.app.layers[0].at.row, 2);
    assert_eq!(p.app.layers[0].band.start, 0);
    p.keys("j");
    assert_eq!(p.app.scroll, 1);
    // The image stays inside the content area: it is cut, not moved up
    // under the title.
    let layer = &p.app.layers[0];
    assert_eq!((layer.at.row, layer.band.start), (2, 1));
    assert_eq!(layer.at.rows, layer.band.end - layer.band.start);
}

#[test]
fn png_embeds_get_an_image_layer_with_graphics() {
    // index.md embeds img.png (2x2 px): one cell.
    let mut p = Pane::new(&fixture("vault-basic"), 100, 30);
    p.app.set_cell_px(Some((8, 16)));
    p.key(KeyCode::End);
    p.keys("\n");
    let imgs: Vec<_> = p
        .app
        .layers
        .iter()
        .filter(|l| l.name.starts_with("img-"))
        .collect();
    assert_eq!(imgs.len(), 1, "{:?}", p.app.layers);
    assert_eq!((imgs[0].at.cols, imgs[0].at.rows), (1, 1));
    assert!(!p.screen().contains("[image:"));

    // Without graphics, the placeholder.
    let mut p = Pane::new(&fixture("vault-basic"), 100, 30);
    p.key(KeyCode::End);
    p.keys("\n");
    assert!(p.app.layers.is_empty());
    assert!(p.screen().contains("[image:\u{a0}img.png]"));
}

#[test]
fn a_png_over_8_mb_keeps_the_placeholder() {
    let root = temp_copy("vault-basic", "pane-bigpng");
    let mut big = std::fs::read(root.join("img.png")).unwrap()[..24].to_vec();
    big.resize(9 * 1024 * 1024, 0);
    std::fs::write(root.join("img.png"), big).unwrap();
    let mut p = Pane::new(&root, 100, 30);
    p.app.set_cell_px(Some((8, 16)));
    p.key(KeyCode::End);
    p.keys("\n");
    assert!(p.app.layers.is_empty());
    assert!(p.screen().contains("[image:\u{a0}img.png]"));
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn peek_mode_shows_one_note_at_its_fragment() {
    let mut p = Pane::new(&fixture("vault-basic"), 80, 6);
    p.app.peek = true;
    p.app.open_at("alpha.md", Some("Second Section"));
    let rows = p.draw();
    // No list: the note starts at the left edge, after the panel's border.
    assert!(rows[1].starts_with("│ alpha.md"), "{rows:?}");
    assert!(rows[2].contains("## Second Section"), "{rows:?}");
    // tab does nothing; [ has no summary page to go back to.
    p.keys("\t[");
    assert_eq!(p.app.mode, Mode::Tree);
    assert_eq!(p.app.page(), &Page::Note("alpha.md".into()));
    // Links still work inside the popup.
    p.keys("n\n");
    assert_eq!(p.app.page(), &Page::Note("index.md".into()));
    p.key(KeyCode::Esc);
    assert!(p.app.quit);
}
