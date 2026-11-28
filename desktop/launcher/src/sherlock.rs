//! The panel « Demande à Sherlock » (lot A-19), in the launcher: Tab hands the query over to
//! Sherlock, or « Demander à Sherlock » is chosen. Each request runs `sherlockd --ask`, without a
//! shell; the request and the conversation so far go on its standard input, never in its
//! arguments. Its events (`docs/sherlockd.md`, « The shell's conversation ») come back as JSON
//! lines, read on a thread of their own, bounded, and the panel shows them as they come:
//! Sherlock's texts, the plan of the request (each step, and its state as the consent broker
//! decides it), the answer, or why there is none. The consent window stays the compositor's:
//! the panel only shows.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use iced::futures::channel::mpsc;
use serde_json::{Value, json};
use sherlock_ui::launcher::sherlock::{self as tokens, text};

/// Longest line read from sherlockd; a longer one ends the reading.
const MAX_LINE: u64 = 256 * 1024;
/// Events read for one request, at most.
const MAX_EVENTS: usize = 10_000;
/// Exchanges kept, and handed to sherlockd for its recap.
pub const MAX_EXCHANGES: usize = 20;

/// A step of the plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub id: String,
    pub tool: String,
    pub summary: String,
    pub state: String,
    /// Run at the read level, with no consent asked.
    pub read: bool,
}

/// A request and what came back for it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Exchange {
    pub request: String,
    pub texts: Vec<String>,
    pub steps: Vec<Step>,
    pub answer: Option<String>,
    /// No answer: what to say.
    pub failure: Option<String>,
    /// When it ended, on the clock of the panel: Sherlock's mood follows it.
    pub ended: Option<Duration>,
}

impl Exchange {
    pub fn running(&self) -> bool {
        self.ended.is_none()
    }
}

/// The conversation: the exchanges, the last one running or not.
#[derive(Debug, Default)]
pub struct Conversation {
    pub exchanges: Vec<Exchange>,
    child: Option<Child>,
}

impl Conversation {
    pub fn running(&self) -> bool {
        self.exchanges.last().is_some_and(Exchange::running)
    }

    /// What sherlockd is handed for `request`: it, and the exchanges answered so far.
    pub fn input(&self, request: &str) -> String {
        let history: Vec<Value> = self
            .exchanges
            .iter()
            .filter_map(|e| Some(json!([e.request, e.answer.as_ref()?])))
            .collect();
        json!({ "request": request, "history": history }).to_string()
    }

    /// Starts `request` with `program --ask`: the events come on the returned receiver. A
    /// program that does not start ends the exchange at once, with why.
    pub fn ask(
        &mut self,
        program: &PathBuf,
        request: &str,
        now: Duration,
    ) -> Option<mpsc::UnboundedReceiver<Value>> {
        let input = self.input(request);
        self.exchanges.push(Exchange {
            request: request.to_owned(),
            ..Exchange::default()
        });
        let skip = self.exchanges.len().saturating_sub(MAX_EXCHANGES);
        self.exchanges.drain(..skip);
        self.stop();
        match spawn(program, input) {
            Ok((child, events)) => {
                self.child = Some(child);
                Some(events)
            }
            Err(error) => {
                tracing::warn!(%error, "sherlockd does not start");
                self.end(text::NO_SHERLOCKD.to_owned(), now);
                None
            }
        }
    }

    /// An event of the request running.
    pub fn apply(&mut self, event: &Value, now: Duration) {
        let Some(exchange) = self.exchanges.last_mut().filter(|e| e.running()) else {
            return;
        };
        let field = |name: &str| event.get(name).and_then(Value::as_str).unwrap_or_default();
        match field("event") {
            "text" => exchange.texts.push(field("text").to_owned()),
            "step" => exchange.steps.push(Step {
                id: field("id").to_owned(),
                tool: field("tool").to_owned(),
                summary: field("summary").to_owned(),
                state: field("state").to_owned(),
                read: false,
            }),
            "state" => {
                if let Some(step) = exchange.steps.iter_mut().find(|s| s.id == field("id")) {
                    step.state = field("state").to_owned();
                    if let Some(level) = event.get("level").and_then(Value::as_str) {
                        step.read = level == "lecture";
                    }
                }
            }
            "answer" => {
                exchange.answer = Some(field("text").to_owned());
                exchange.ended = Some(now);
            }
            "failed" => {
                let said = match field("reason") {
                    "offline" => text::OFFLINE.to_owned(),
                    // sherlockd's own words: what to install, or how to log in.
                    "not-installed" | "not-logged-in" => field("message").to_owned(),
                    _ => text::FAILED_ANSWER.to_owned(),
                };
                self.end(said, now);
            }
            _ => {}
        }
    }

    /// sherlockd stopped writing: an exchange still running ends without an answer.
    pub fn closed(&mut self, now: Duration) {
        if self.running() {
            self.end(text::FAILED_ANSWER.to_owned(), now);
        }
        self.stop();
    }

    fn end(&mut self, failure: String, now: Duration) {
        if let Some(exchange) = self.exchanges.last_mut().filter(|e| e.running()) {
            exchange.failure = Some(failure);
            exchange.ended = Some(now);
        }
    }

    /// Stops the request running, if any: its process goes.
    pub fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = std::thread::Builder::new()
                .name("sherlockd-reap".into())
                .spawn(move || child.wait());
        }
    }

    /// Sherlock's mood at `now`: thinking while a request runs, happy or sorry for a while
    /// after it ended, else at rest.
    pub fn mood(&self, now: Duration) -> sherlock_3d::Mood {
        use sherlock_3d::Mood;
        let Some(last) = self.exchanges.last() else {
            return Mood::Idle;
        };
        match last.ended {
            None => Mood::Think,
            Some(at) if now < at + tokens::MOOD_HOLD => {
                if last.failure.is_some() {
                    Mood::Sorry
                } else {
                    Mood::Success
                }
            }
            Some(_) => Mood::Idle,
        }
    }
}

impl Drop for Conversation {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The panel, `width` wide: Sherlock in his bubble and the title, the conversation (scrolled to
/// its end), the field of the next request. `field` names the field, for the focus.
pub fn view<'a, M: Clone + 'a>(
    conversation: &'a Conversation,
    request: &'a str,
    model: Option<std::sync::Arc<sherlock_3d::Model>>,
    now: Duration,
    on_input: impl Fn(String) -> M + 'a,
    on_submit: M,
    field: iced::widget::Id,
) -> iced::Element<'a, M> {
    use iced::widget::{Space, column, container, row, scrollable, text, text_input};
    use iced::{Alignment, Length};
    use sherlock_ui::component as ui;
    use sherlock_ui::launcher as look;
    use sherlock_ui::{color, typography};
    let p = color::palette();
    let bubble = container(
        sherlock_3d::Sherlock3d::new(model)
            .mood(conversation.mood(now))
            .width(tokens::BUBBLE)
            .height(tokens::BUBBLE),
    )
    .width(tokens::BUBBLE)
    .height(tokens::BUBBLE)
    .clip(true);
    let title = column![
        text(tokens::text::TITLE)
            .size(look::QUERY_SIZE)
            .font(typography::UI_SEMIBOLD),
        ui::ellipsis(tokens::text::SUBTITLE)
            .size(tokens::SMALL_SIZE)
            .color(p.text_secondary)
            .width(Length::Fill),
    ]
    .spacing(2);
    let header = row![bubble, title]
        .spacing(look::GAP)
        .align_y(Alignment::Center)
        .padding([0.0, look::SEARCH_PADDING])
        .height(tokens::HEADER_HEIGHT);
    let words = |said: &'a str, tint: color::Color| {
        text(said)
            .size(tokens::TEXT_SIZE)
            .color(tint)
            .wrapping(text::Wrapping::WordOrGlyph)
    };
    let mut body = column![].spacing(tokens::GAP).padding(tokens::PADDING);
    if conversation.exchanges.is_empty() {
        body = body.push(words(tokens::text::EMPTY, p.text_secondary));
    }
    for exchange in &conversation.exchanges {
        let asked = container(words(&exchange.request, p.text_primary))
            .padding(tokens::STEP_PADDING)
            .style(look::request);
        body = body.push(container(asked).align_right(Length::Fill));
        for said in &exchange.texts {
            body = body.push(words(said, p.text_primary));
        }
        if !exchange.steps.is_empty() {
            body = body.push(ui::section_label(tokens::text::PLAN));
        }
        for (place, step) in exchange.steps.iter().enumerate() {
            let number = container(text((place + 1).to_string()).size(tokens::SMALL_SIZE))
                .center(tokens::STEP_NUMBER)
                .style(look::key(false));
            let lines = column![
                ui::ellipsis(step.tool.as_str()).font(typography::UI_SEMIBOLD),
                ui::ellipsis(step.summary.as_str())
                    .size(tokens::SMALL_SIZE)
                    .color(p.text_secondary),
            ]
            .width(Length::Fill);
            let state = text(tokens::state_line(&step.state, step.read))
                .size(tokens::SMALL_SIZE)
                .color(p.text_secondary);
            let card = row![number, lines, state]
                .spacing(look::GAP)
                .align_y(Alignment::Center);
            let waiting = step.state == "waiting";
            body = body.push(
                container(card)
                    .padding(tokens::STEP_PADDING)
                    .width(Length::Fill)
                    .style(look::step(waiting)),
            );
        }
        if let Some(answer) = &exchange.answer {
            body = body.push(words(answer, p.text_primary));
        }
        if let Some(failure) = &exchange.failure {
            body = body.push(words(failure, p.text_secondary));
        }
        if exchange.running() {
            body = body.push(words(tokens::text::THINKING, p.text_tertiary));
        }
    }
    let talk = scrollable(body)
        .anchor_bottom()
        .height(Length::Fill)
        .width(Length::Fill);
    let mut input = text_input(tokens::text::FIELD, request)
        .id(field)
        .size(look::QUERY_SIZE)
        .padding(0)
        .style(look::query);
    if !conversation.running() {
        input = input.on_input(on_input).on_submit(on_submit);
    }
    let ask = row![
        ui::icon(
            sherlock_ui::icon::Icon::Search,
            look::SEARCH_ICON,
            p.accent_text
        ),
        input,
        Space::new().width(0),
    ]
    .spacing(look::GAP)
    .align_y(Alignment::Center)
    .padding([0.0, look::SEARCH_PADDING])
    .height(look::SEARCH_HEIGHT);
    column![header, talk, ask].height(Length::Fill).into()
}

/// The program of Sherlock's service: beside the session's programs, or in the `PATH`.
pub fn program() -> PathBuf {
    std::env::var_os("SHERLOCKOS_BIN")
        .map(|dir| PathBuf::from(dir).join("sherlockd"))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from("sherlockd"))
}

/// Starts `program --ask`, writes `input` on its standard input and closes it, and reads its
/// events on a thread of its own.
fn spawn(
    program: &PathBuf,
    input: String,
) -> std::io::Result<(Child, mpsc::UnboundedReceiver<Value>)> {
    let mut child = Command::new(program)
        .arg("--ask")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let missing = || std::io::Error::other("no pipe");
    let mut stdin = child.stdin.take().ok_or_else(missing)?;
    let stdout = child.stdout.take().ok_or_else(missing)?;
    let (sender, receiver) = mpsc::unbounded();
    std::thread::Builder::new()
        .name("sherlockd-ask".into())
        .spawn(move || {
            // Written here: a long request never waits on the interface thread.
            if let Err(error) = stdin.write_all(input.as_bytes()) {
                tracing::warn!(%error, "request not written");
            }
            drop(stdin);
            let mut reader = BufReader::new(stdout);
            for _ in 0..MAX_EVENTS {
                let mut line = Vec::new();
                match (&mut reader).take(MAX_LINE).read_until(b'\n', &mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) if !line.ends_with(b"\n") => break,
                    Ok(_) => {}
                }
                let Ok(event) = serde_json::from_slice::<Value>(&line) else {
                    continue;
                };
                if sender.unbounded_send(event).is_err() {
                    break;
                }
            }
        })?;
    Ok((child, receiver))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sherlock_3d::Mood;

    fn running(request: &str) -> Conversation {
        let mut conversation = Conversation::default();
        conversation.exchanges.push(Exchange {
            request: request.into(),
            ..Exchange::default()
        });
        conversation
    }

    #[test]
    fn the_events_fill_the_exchange_and_the_mood_follows() {
        let mut c = running("Relance front-next");
        let at = Duration::from_secs;
        assert_eq!(c.mood(at(1)), Mood::Think);
        c.apply(&json!({"event": "text", "text": "Je regarde."}), at(1));
        c.apply(
            &json!({"event": "step", "id": "a", "tool": "restart", "summary": "{}", "state": "waiting"}),
            at(1),
        );
        c.apply(
            &json!({"event": "state", "id": "a", "state": "granted", "level": "action"}),
            at(2),
        );
        c.apply(
            &json!({"event": "state", "id": "a", "state": "done"}),
            at(3),
        );
        c.apply(&json!({"event": "answer", "text": "C'est fait."}), at(4));
        let e = &c.exchanges[0];
        assert_eq!(e.texts, ["Je regarde."]);
        assert_eq!(
            (e.steps[0].state.as_str(), e.steps[0].read),
            ("done", false)
        );
        assert_eq!(e.answer.as_deref(), Some("C'est fait."));
        assert_eq!(c.mood(at(5)), Mood::Success);
        assert_eq!(c.mood(at(10)), Mood::Idle);
        // An event after the end changes nothing.
        c.apply(&json!({"event": "text", "text": "late"}), at(11));
        assert_eq!(c.exchanges[0].texts.len(), 1);
        // The next request carries the answered one.
        let input: Value = serde_json::from_str(&c.input("Et ensuite ?")).unwrap();
        assert_eq!(input["history"][0][1], "C'est fait.");
    }

    #[test]
    fn without_an_answer_the_reason_is_said() {
        let mut c = running("Bonjour");
        c.apply(
            &json!({"event": "failed", "reason": "offline", "message": "x"}),
            Duration::ZERO,
        );
        assert_eq!(c.exchanges[0].failure.as_deref(), Some(text::OFFLINE));
        assert_eq!(c.mood(Duration::from_secs(1)), Mood::Sorry);
        let mut c = running("Bonjour");
        c.closed(Duration::ZERO);
        assert_eq!(c.exchanges[0].failure.as_deref(), Some(text::FAILED_ANSWER));
        // sherlockd missing: said at once.
        let mut c = Conversation::default();
        let events = c.ask(
            &PathBuf::from("/nonexistent/sherlockd"),
            "Bonjour",
            Duration::ZERO,
        );
        assert!(events.is_none());
        assert_eq!(c.exchanges[0].failure.as_deref(), Some(text::NO_SHERLOCKD));
    }

    #[test]
    fn a_fake_sherlockd_is_read_line_by_line_and_never_gets_the_request_in_argv() {
        let dir = std::env::temp_dir().join(format!("launcher-ask-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("sherlockd");
        let seen = dir.join("seen");
        // Writes its arguments and its input, then two events.
        let script = format!(
            "#!/bin/sh\necho \"$@\" > '{s}.args'\ncat > '{s}.input'\n\
             echo '{{\"event\":\"text\",\"text\":\"Bonjour\"}}'\n\
             echo '{{\"event\":\"answer\",\"text\":\"Fini\"}}'\n",
            s = seen.display()
        );
        std::fs::write(&fake, script).unwrap();
        let status = Command::new("chmod").arg("+x").arg(&fake).status().unwrap();
        assert!(status.success());
        let mut c = Conversation::default();
        let mut events = c.ask(&fake, "secret", Duration::ZERO).unwrap();
        let mut next =
            || iced::futures::executor::block_on(iced::futures::StreamExt::next(&mut events));
        let first = next().unwrap();
        c.apply(&first, Duration::ZERO);
        let second = next().unwrap();
        c.apply(&second, Duration::ZERO);
        assert_eq!(c.exchanges[0].answer.as_deref(), Some("Fini"));
        assert!(next().is_none(), "the end of the output ends the stream");
        let args = std::fs::read_to_string(dir.join("seen.args")).unwrap();
        assert_eq!(args.trim(), "--ask");
        let input = std::fs::read_to_string(dir.join("seen.input")).unwrap();
        assert!(input.contains("secret"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
