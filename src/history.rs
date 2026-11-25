//! cliphist interaction: listing, parsing, decoding, and spawning copy/paste.

use anyhow::{Context, Result, bail};
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use std::io::Write;
use std::process::{Command, Stdio};

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Text,
    Image {
        format: String,
        dims: String,
        size: String,
    },
    Binary {
        size: String,
    },
}

/// One `cliphist list` line: `<id>\t<preview>`.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// The full line, as `cliphist decode` expects it on stdin.
    pub line: String,
    pub id: String,
    pub preview: String,
    pub kind: Kind,
}

impl Entry {
    pub fn is_image(&self) -> bool {
        matches!(self.kind, Kind::Image { .. })
    }

    /// Single-line text for the list row.
    pub fn label(&self) -> String {
        match &self.kind {
            Kind::Text => self.preview.clone(),
            Kind::Image { format, dims, size } => format!("{format} {dims} · {size}"),
            Kind::Binary { size } => format!("binary · {size}"),
        }
    }
}

pub fn parse_line(line: &str) -> Option<Entry> {
    let (id, preview) = line.split_once('\t')?;
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(Entry {
        line: line.to_string(),
        id: id.to_string(),
        preview: preview.to_string(),
        kind: parse_kind(preview),
    })
}

/// cliphist previews binary entries as `[[ binary data <n> <unit> [<fmt> <w>x<h>] ]]`.
fn parse_kind(preview: &str) -> Kind {
    let Some(inner) = preview
        .strip_prefix("[[ binary data ")
        .and_then(|s| s.strip_suffix(" ]]"))
    else {
        return Kind::Text;
    };
    let tokens: Vec<&str> = inner.split_whitespace().collect();
    match tokens.as_slice() {
        [n, unit, format, dims, ..] => Kind::Image {
            format: format.to_string(),
            dims: dims.replace('x', "×"),
            size: format!("{n} {unit}"),
        },
        [n, unit, ..] => Kind::Binary {
            size: format!("{n} {unit}"),
        },
        _ => Kind::Binary {
            size: inner.to_string(),
        },
    }
}

pub fn parse_list(output: &str) -> Vec<Entry> {
    output.lines().filter_map(parse_line).collect()
}

pub fn list() -> Result<Vec<Entry>> {
    let out = Command::new("cliphist")
        .arg("list")
        .stderr(Stdio::null())
        .output()
        .context("running cliphist list")?;
    if !out.status.success() {
        bail!("cliphist list failed: {}", out.status);
    }
    Ok(parse_list(&String::from_utf8_lossy(&out.stdout)))
}

/// Indices of entries matching `query` (fuzzy, smart case): best matches
/// first, ties in history order. An empty query keeps the whole history.
pub fn filter(entries: &[Entry], query: &str) -> Vec<usize> {
    if query.trim().is_empty() {
        return (0..entries.len()).collect();
    }
    let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
    let mut matcher = Matcher::new(Config::DEFAULT);
    let mut buf = Vec::new();
    let mut hits: Vec<(usize, u32)> = entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            let label = e.label();
            pattern
                .score(Utf32Str::new(&label, &mut buf), &mut matcher)
                .map(|s| (i, s))
        })
        .collect();
    hits.sort_by_key(|&(_, score)| std::cmp::Reverse(score)); // stable: ties stay in history order
    hits.into_iter().map(|(i, _)| i).collect()
}

fn feed(cmd: &mut Command, input: &str) -> Result<std::process::Child> {
    let mut child = cmd.stdin(Stdio::piped()).spawn()?;
    let mut stdin = child.stdin.take().context("no stdin")?;
    stdin.write_all(input.as_bytes())?;
    drop(stdin);
    Ok(child)
}

pub fn decode(line: &str) -> Result<Vec<u8>> {
    let child = feed(
        Command::new("cliphist")
            .arg("decode")
            .stdout(Stdio::piped())
            .stderr(Stdio::null()),
        &format!("{line}\n"),
    )
    .context("running cliphist decode")?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!("cliphist decode failed: {}", out.status);
    }
    Ok(out.stdout)
}

/// Reap a child in the background so it doesn't linger as a zombie.
fn reap(child: std::process::Child) {
    std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });
}

/// Put an entry on the clipboard. cliphist re-stores it, so it becomes newest.
pub fn copy(line: &str) -> Result<()> {
    let child = feed(
        Command::new("sh").args(["-c", "cliphist decode | wl-copy"]),
        &format!("{line}\n"),
    )?;
    reap(child);
    Ok(())
}

/// Paste entries into the focused window via the hypr-clipboard-paste script.
pub fn paste(lines: &[&str]) -> Result<()> {
    let script = std::env::var_os("HOME")
        .map(|h| std::path::PathBuf::from(h).join(".local/bin/hypr-clipboard-paste"))
        .context("HOME is unset")?;
    let mut input = lines.join("\n");
    input.push('\n');
    let child = feed(&mut Command::new(script), &input)?;
    reap(child);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_text_line() {
        let e = parse_line("3299\thttps://kepigame.com/games/nenrei.html").unwrap();
        assert_eq!(e.id, "3299");
        assert_eq!(e.kind, Kind::Text);
        assert_eq!(e.label(), "https://kepigame.com/games/nenrei.html");
    }

    #[test]
    fn keeps_tabs_inside_text() {
        let e = parse_line("12\ta\tb").unwrap();
        assert_eq!(e.preview, "a\tb");
        assert_eq!(e.line, "12\ta\tb");
    }

    #[test]
    fn parses_image_line() {
        let e = parse_line("3300\t[[ binary data 146 KiB png 755x1061 ]]").unwrap();
        assert_eq!(
            e.kind,
            Kind::Image {
                format: "png".into(),
                dims: "755×1061".into(),
                size: "146 KiB".into()
            }
        );
        assert!(e.is_image());
        assert_eq!(e.label(), "png 755×1061 · 146 KiB");
    }

    #[test]
    fn parses_non_image_binary() {
        let e = parse_line("7\t[[ binary data 10 KiB ]]").unwrap();
        assert_eq!(
            e.kind,
            Kind::Binary {
                size: "10 KiB".into()
            }
        );
        assert!(!e.is_image());
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_line("no tab here").is_none());
        assert!(parse_line("abc\tpreview").is_none());
        assert!(parse_line("\tpreview").is_none());
    }

    #[test]
    fn parse_list_skips_bad_lines() {
        let entries = parse_list("2\tb\nbad\n1\ta\n");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1].id, "1");
    }

    #[test]
    fn filter_ranks_best_match_first() {
        let entries = parse_list("3\tMike's Pizza\n2\tunrelated\n1\thttps://kepigame.com\n");
        assert_eq!(filter(&entries, "kepi"), vec![2, 0]);
    }

    #[test]
    fn filter_ties_keep_history_order() {
        let entries = parse_list(
            "4\thttps://x.com/telegram\n3\t[[ binary data 1 KiB png 1x1 ]]\n2\tsudo pacman\n1\tx telegram\n",
        );
        assert_eq!(filter(&entries, ""), vec![0, 1, 2, 3]);
        assert_eq!(filter(&entries, "sudo"), vec![2]);
        let same = parse_list("3\ttelegram\n2\tother\n1\ttelegram\n");
        assert_eq!(filter(&same, "telegram"), vec![0, 2]);
        assert_eq!(filter(&entries, "png"), vec![1]);
        assert_eq!(filter(&entries, "zzzz"), Vec::<usize>::new());
    }

    #[test]
    fn filter_is_smart_case() {
        let entries = parse_list("2\tHello\n1\thello\n");
        assert_eq!(filter(&entries, "hello"), vec![0, 1]);
        assert_eq!(filter(&entries, "Hello"), vec![0]);
    }
}
