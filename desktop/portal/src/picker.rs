//! The picker: a process of its own (`sherlock-portal --pick`), a window of the shell where the
//! user chooses a screen or a window. The portal writes it the request on its standard input,
//! one line each: the application, what it may share, then each source; the picker answers on
//! its standard output with the numbers of the sources chosen, one a line, or nothing when the
//! user cancels. Fields are separated by tabs, which names never hold here (they are replaced).

use std::{
    io::{BufRead as _, BufReader, Read as _, Write as _},
    process::{Command, Stdio},
};

use crate::{
    capture::{Screen, Source, Window},
    screencast::{MONITOR, Selection, WINDOW},
};

/// The longest answer read from the picker.
const MAX_ANSWER: u64 = 4096;
/// The longest field kept from the request.
const MAX_FIELD: usize = 512;

/// What the picker is asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub app_id: String,
    pub selection: Selection,
    pub sources: Vec<Source>,
}

/// A field on one line, without tabs nor line ends, bounded.
fn field(text: &str) -> String {
    let clean = text.replace(['\t', '\n', '\r'], " ");
    clean.chars().take(MAX_FIELD).collect()
}

impl Request {
    pub fn encode(&self) -> String {
        let s = &self.selection;
        let mut out = format!("app\t{}\n", field(&self.app_id));
        out.push_str(&format!(
            "select\t{}\t{}\t{}\n",
            s.types,
            u8::from(s.multiple),
            u8::from(s.cursor)
        ));
        for source in &self.sources {
            match source {
                Source::Screen(screen) => out.push_str(&format!(
                    "screen\t{}\t{}\n",
                    field(&screen.name),
                    field(&screen.description)
                )),
                Source::Window(window) => out.push_str(&format!(
                    "window\t{}\t{}\t{}\n",
                    field(&window.identifier),
                    field(&window.app_id),
                    field(&window.title)
                )),
            }
        }
        out
    }

    /// Reads a request; lines it does not know are left out.
    pub fn decode(text: &str) -> Self {
        let mut request = Request {
            app_id: String::new(),
            selection: Selection::default(),
            sources: Vec::new(),
        };
        for line in text.lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            match parts.as_slice() {
                ["app", app_id] => request.app_id = field(app_id),
                ["select", types, multiple, cursor] => {
                    request.selection = Selection {
                        types: types.parse::<u32>().unwrap_or(MONITOR) & (MONITOR | WINDOW),
                        multiple: *multiple == "1",
                        cursor: *cursor == "1",
                    }
                }
                ["screen", name, description] => request.sources.push(Source::Screen(Screen {
                    name: field(name),
                    description: field(description),
                })),
                ["window", identifier, app_id, title] => {
                    request.sources.push(Source::Window(Window {
                        identifier: field(identifier),
                        app_id: field(app_id),
                        title: field(title),
                    }))
                }
                _ => {}
            }
        }
        request
    }

    /// The sources the user may choose from: those of the kinds asked for.
    pub fn offered(&self) -> impl Iterator<Item = (usize, &Source)> {
        let types = self.selection.types;
        self.sources
            .iter()
            .enumerate()
            .filter(move |(_, s)| match s {
                Source::Screen(_) => types & MONITOR != 0,
                Source::Window(_) => types & WINDOW != 0,
            })
    }
}

/// The sources the answer `text` chose among those offered by `request`: none if it chose
/// none or something not offered, one unless several were allowed.
pub fn chosen(request: &Request, text: &str) -> Option<Vec<Source>> {
    let mut chosen = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let index: usize = line.trim().parse().ok()?;
        let (_, source) = request.offered().find(|(i, _)| *i == index)?;
        if !chosen.contains(source) {
            chosen.push(source.clone());
        }
    }
    let allowed = if request.selection.multiple {
        usize::MAX
    } else {
        1
    };
    (!chosen.is_empty() && chosen.len() <= allowed).then_some(chosen)
}

/// Asks the user which of `sources` to share for `app_id`. `None`: cancelled, or no picker.
pub fn pick(app_id: &str, selection: &Selection, sources: &[Source]) -> Option<Vec<Source>> {
    let request = Request {
        app_id: app_id.to_owned(),
        selection: selection.clone(),
        sources: sources.to_vec(),
    };
    let program = std::env::current_exe().ok()?;
    let mut child = Command::new(program)
        .arg("--pick")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .inspect_err(|err| tracing::warn!("picker: {err}"))
        .ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(request.encode().as_bytes());
    }
    let mut answer = String::new();
    if let Some(stdout) = child.stdout.take() {
        let _ = BufReader::new(stdout.take(MAX_ANSWER)).read_to_string(&mut answer);
    }
    let _ = child.wait();
    chosen(&request, &answer)
}

/// The request the picker reads on its standard input, bounded.
pub fn read_request() -> Request {
    let mut text = String::new();
    let stdin = std::io::stdin().lock();
    for line in BufReader::new(stdin)
        .take(1 << 20)
        .lines()
        .map_while(Result::ok)
    {
        text.push_str(&line);
        text.push('\n');
    }
    Request::decode(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> Request {
        Request {
            app_id: String::from("com.discordapp.Discord"),
            selection: Selection {
                types: MONITOR | WINDOW,
                multiple: false,
                cursor: true,
            },
            sources: vec![
                Source::Screen(Screen {
                    name: String::from("eDP-1"),
                    description: String::from("Built-in\tscreen"),
                }),
                Source::Window(Window {
                    identifier: String::from("a1b2"),
                    app_id: String::from("firefox"),
                    title: String::from("Notes\nand more"),
                }),
            ],
        }
    }

    #[test]
    fn a_request_reads_back_without_tabs_nor_line_ends() {
        let decoded = Request::decode(&request().encode());
        assert_eq!(decoded.app_id, "com.discordapp.Discord");
        assert_eq!(decoded.selection, request().selection);
        let Source::Window(window) = &decoded.sources[1] else {
            panic!("a window");
        };
        assert_eq!(window.title, "Notes and more");
        let Source::Screen(screen) = &decoded.sources[0] else {
            panic!("a screen");
        };
        assert_eq!(screen.description, "Built-in screen");
    }

    #[test]
    fn the_answer_chooses_among_what_is_offered() {
        let mut request = request();
        assert_eq!(
            chosen(&request, "1\n"),
            Some(vec![request.sources[1].clone()])
        );
        // Nothing, garbage, a number out of range, or two when one is allowed: cancelled.
        for answer in ["", "x", "7", "0\n1\n"] {
            assert_eq!(chosen(&request, answer), None, "{answer:?}");
        }
        // Screens only: the window is not offered.
        request.selection.types = MONITOR;
        assert_eq!(chosen(&request, "1"), None);
        request.selection.multiple = true;
        assert_eq!(chosen(&request, "0\n0\n").map(|c| c.len()), Some(1));
    }
}
