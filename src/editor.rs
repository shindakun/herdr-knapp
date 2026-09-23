//! Choosing and invoking the editor, and OSC 52 copies.

use std::path::Path;

/// Config `editor`, then `$VISUAL`, then `$EDITOR`, then `vi`.
pub fn choose(config: &str, visual: Option<&str>, editor: Option<&str>) -> String {
    [Some(config), visual, editor]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|e| !e.is_empty())
        .unwrap_or("vi")
        .to_string()
}

/// The argv that opens `path` at `line`. The editor string is split on
/// whitespace; quoting is not supported.
pub fn command(editor: &str, path: &Path, line: u32) -> Vec<String> {
    let mut argv: Vec<String> = editor.split_whitespace().map(str::to_string).collect();
    let file = path.display().to_string();
    let name = argv
        .first()
        .map(|c| c.rsplit('/').next().unwrap_or(c).to_string())
        .unwrap_or_default();
    match name.as_str() {
        "vi" | "vim" | "nvim" | "nano" | "emacs" | "micro" | "kak" => {
            argv.push(format!("+{line}"));
            argv.push(file);
        }
        "hx" => argv.push(format!("{file}:{line}")),
        "code" => {
            argv.push("-g".into());
            argv.push(format!("{file}:{line}"));
        }
        _ => argv.push(file),
    }
    argv
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The OSC 52 sequence that sets the clipboard to `text`.
pub fn osc52(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", base64(text.as_bytes()))
}
