use std::path::Path;

use knapp::editor::{base64, choose, command, osc52};

#[test]
fn line_arguments_by_editor() {
    let p = Path::new("/n/a.md");
    assert_eq!(command("vi", p, 7), ["vi", "+7", "/n/a.md"]);
    assert_eq!(
        command("/usr/bin/nvim", p, 7),
        ["/usr/bin/nvim", "+7", "/n/a.md"]
    );
    assert_eq!(command("hx", p, 7), ["hx", "/n/a.md:7"]);
    assert_eq!(command("code -w", p, 7), ["code", "-w", "-g", "/n/a.md:7"]);
    assert_eq!(command("subl", p, 7), ["subl", "/n/a.md"]);
    assert_eq!(command("nano -l", p, 3), ["nano", "-l", "+3", "/n/a.md"]);
}

#[test]
fn choice_order() {
    assert_eq!(choose("hx", Some("vim"), Some("nano")), "hx");
    assert_eq!(choose("", Some("vim"), Some("nano")), "vim");
    assert_eq!(choose(" ", None, Some("nano")), "nano");
    assert_eq!(choose("", None, None), "vi");
}

#[test]
fn base64_and_osc52() {
    assert_eq!(base64(b""), "");
    assert_eq!(base64(b"f"), "Zg==");
    assert_eq!(base64(b"fo"), "Zm8=");
    assert_eq!(base64(b"foo"), "Zm9v");
    assert_eq!(base64("/n/café.md".as_bytes()), "L24vY2Fmw6kubWQ=");
    assert_eq!(osc52("foo"), "\x1b]52;c;Zm9v\x07");
}
