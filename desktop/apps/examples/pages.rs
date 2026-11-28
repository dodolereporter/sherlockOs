//! Measures the view's animations (lot P-02): opens the view, turns pages forward and back, then
//! closes it, by itself, and logs the frames of each animation (`startup::Frames`) and how many
//! SVG icons iced rasterized meanwhile. Run it in a Wayland session that nobody uses, a headless
//! compositor for instance: `cargo run --release -p apps --example pages`.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use apps::app::{Message, View};
use iced::futures::Stream;
use iced::futures::channel::mpsc;
use iced::mouse::ScrollDelta;
use iced::window::Id;
use iced::{Element, Subscription, Task};
use iced_exwlshell::settings::ExWlSettings;
use iced_exwlshell::shell::{ShellInfo, ShellType};
use launcher::service::Request;
use sherlock_ui::{color, theme};

/// SVG pictures iced rasterized: it logs each one (`allocating`, at the debug level).
static RASTERIZED: AtomicUsize = AtomicUsize::new(0);

struct Counter;

impl log::Log for Counter {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.target().starts_with("iced_wgpu")
    }

    fn log(&self, record: &log::Record<'_>) {
        if record.target().starts_with("iced_wgpu")
            && record.args().to_string().starts_with("allocating")
        {
            RASTERIZED.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn flush(&self) {}
}

static COUNTER: Counter = Counter;

/// What the autopilot does, in order, one step every 700 ms.
#[derive(Debug, Clone, Copy)]
enum Step {
    Open,
    Next,
    Back,
    Close,
    Report,
    Quit,
}

const SCRIPT: [Step; 14] = [
    Step::Open,
    Step::Report,
    Step::Next,
    Step::Report,
    Step::Next,
    Step::Report,
    Step::Back,
    Step::Report,
    Step::Back,
    Step::Report,
    Step::Close,
    Step::Report,
    Step::Open,
    Step::Quit,
];

fn autopilot() -> impl Stream<Item = Step> {
    let (sender, receiver) = mpsc::unbounded();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(2));
        for step in SCRIPT {
            if sender.unbounded_send(step).is_err() {
                return;
            }
            std::thread::sleep(Duration::from_millis(700));
        }
    });
    receiver
}

#[iced_exwlshell::to_layer_message(multi)]
#[derive(Debug, Clone)]
enum Bench {
    View(Message),
    Step(Step),
}

/// The view's message, its surface requests handed to the runtime as the bench's own.
fn lift(message: Message) -> Bench {
    match message {
        Message::NewLayerShell { settings, id } => Bench::NewLayerShell { settings, id },
        Message::RemoveWindow(id) => Bench::RemoveWindow(id),
        other => Bench::View(other),
    }
}

struct State {
    view: View,
    /// CPU time of the process at the last report, in milliseconds.
    cpu: u64,
}

/// CPU time of this process (user and system), in milliseconds, from `/proc/self/stat`.
fn cpu_ms() -> u64 {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    let fields: Vec<&str> = stat
        .rsplit_once(')')
        .map(|(_, rest)| rest.split_whitespace().collect())
        .unwrap_or_default();
    let ticks: u64 = [11, 12]
        .iter()
        .filter_map(|i| fields.get(*i)?.parse::<u64>().ok())
        .sum();
    // The kernel counts in hundredths of a second on Linux (USER_HZ).
    ticks * 10
}

impl State {
    fn update(&mut self, message: Bench) -> Task<Bench> {
        let message = match message {
            Bench::View(message) => message,
            Bench::Step(Step::Open | Step::Close) => Message::Service(Request::Toggle),
            Bench::Step(Step::Next) => Message::Scroll(ScrollDelta::Lines { x: 0.0, y: -1.0 }),
            Bench::Step(Step::Back) => Message::Scroll(ScrollDelta::Lines { x: 0.0, y: 1.0 }),
            Bench::Step(Step::Report) => {
                let count = RASTERIZED.swap(0, Ordering::Relaxed);
                let cpu = cpu_ms();
                tracing::info!(
                    rasterized = count,
                    drawn = self.view.rasters().icons.len(),
                    cpu_ms = cpu - self.cpu,
                    "since the last step: SVG rasterized by iced, CPU time of the process"
                );
                self.cpu = cpu;
                return Task::none();
            }
            Bench::Step(Step::Quit) => return iced::exit(),
            _ => return Task::none(),
        };
        self.view.update(message).map(lift)
    }

    fn view(&self, id: Id) -> Element<'_, Bench> {
        self.view.view(id).map(Bench::View)
    }

    fn subscription(&self) -> Subscription<Bench> {
        Subscription::batch([
            self.view.subscription().map(Bench::View),
            Subscription::run(autopilot).map(Bench::Step),
        ])
    }
}

fn main() -> Result<(), iced_exwlshell::Error> {
    let renderer = sherlock_ui::gpu::Choice::detect();
    // SAFETY: first statement of `main`, no other thread exists yet.
    unsafe { renderer.apply() };
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();
    let _ = log::set_logger(&COUNTER);
    log::set_max_level(log::LevelFilter::Debug);
    launcher::apply_theme();
    let (_, receiver) = mpsc::unbounded();
    let inbox = launcher::service::Inbox::from_receiver(receiver);
    let reload: apps::app::Reload = Arc::new(apps::catalog);
    let (broadcast, outputs) = iced_exwlshell::shell::channel();
    let first = std::sync::Mutex::new(Some(View::new(
        apps::catalog(),
        Arc::new(|_: &_, _: Option<&str>| Ok(0)),
    )));
    let boot = move || {
        let view = first
            .lock()
            .ok()
            .and_then(|mut v| v.take())
            .unwrap_or_else(|| {
                View::new(apps::catalog(), Arc::new(|_: &_, _: Option<&str>| Ok(0)))
            });
        let mut view = view
            .resident(inbox.clone(), reload.clone())
            .with_outputs(outputs.clone());
        // `PAGES_SVG=1`: the icons drawn by iced's SVG widget, as before lot P-02.
        view.rasterize = std::env::var_os("PAGES_SVG").is_none_or(|v| v.is_empty());
        let mut state = State {
            view,
            cpu: cpu_ms(),
        };
        let task = state.view.warm_up().map(lift);
        (state, task)
    };
    iced_exwlshell::daemon(boot, || String::from("pages"), State::update, State::view)
        .theme(|_: &State, _| Some(theme::theme()))
        .style(|_, _| iced::theme::Style {
            background_color: color::TRANSPARENT.into(),
            text_color: color::palette().text_primary.into(),
        })
        .subscription(State::subscription)
        .on_new_shell(|info: ShellInfo| {
            (info.shell == ShellType::LayerShell)
                .then_some(Bench::View(Message::Surface(info.window)))
        })
        .settings(launcher::settings())
        .wl_settings(ExWlSettings {
            // On the shell socket, the only one that offers anchored surfaces (lot D-46).
            with_connection: Some(shell_protocol::connect.into()),
            layer_settings: iced_exwlshell::settings::LayerShellSettings {
                start_mode: iced_exwlshell::settings::StartMode::Background,
                ..Default::default()
            },
            shell_broadcast: broadcast,
            ..ExWlSettings::default()
        })
        .run()
}
