//! Drag and drop and the clipboard of files, on the side of the application (lot E-03): a press
//! on the selection that becomes a drag out of the window, files dropped on the window (moved,
//! or copied with Ctrl) into the folder under the pointer or the one shown, the folder that
//! lights up and opens when the pointer rests on it, and Ctrl + C, X and V with the other
//! applications. The Wayland side is in `crate::dnd`.

use std::path::PathBuf;
use std::time::Instant;

use iced::{Point, Task};
use sherlock_ui::{color, drag};

use super::{App, Clip, Location, Message, done};
use crate::dnd::{self, Action, Command, Event, Link};
use crate::listing::Kind;
use crate::ops;
use crate::transfer::Mode;
use sherlock_ui::texts::evidence as texts;

/// The application's share of drag and drop.
#[derive(Default)]
pub(crate) struct Dnd {
    link: Option<Link>,
    /// A press on the selection, where it was, and the row to select alone if it stays a click
    /// (pressed on a multiple selection, which a drag carries whole).
    press: Option<(Point, Option<usize>)>,
    /// A drag started here, with what it carries.
    out: Option<Vec<PathBuf>>,
    /// Files dragged over the window.
    pub(crate) over: Option<Over>,
}

/// Files dragged over the window.
pub(crate) struct Over {
    /// The folder row under the pointer, if a drop there is allowed, and since when.
    pub(crate) target: Option<usize>,
    pub(crate) since: Instant,
    /// A drop lands somewhere: in that folder, or in the folder shown.
    pub(crate) allowed: bool,
}

impl Dnd {
    fn send(&self, command: Command) -> bool {
        match &self.link {
            Some(link) => {
                link.send(command);
                true
            }
            None => false,
        }
    }
}

impl App {
    /// The window's Wayland handles: the drag and drop thread starts on them.
    pub(super) fn dnd_ready(&mut self, handles: Option<dnd::Handles>) -> Task<Message> {
        let Some((link, events)) = handles.and_then(dnd::start) else {
            tracing::info!("no drag and drop with the other applications (not on Wayland)");
            return Task::none();
        };
        self.dnd.link = Some(link);
        Task::run(events, Message::Dnd)
    }

    /// A press on the row `index`. `false`: it was a plain press on a row of a multiple
    /// selection, which is kept until the release, for a drag to carry it whole.
    pub(super) fn dnd_press(&mut self, index: usize, plain: bool, on_name: bool) -> bool {
        self.dnd.press = None;
        let movable = matches!(self.location, Location::Folder(_)) && self.dnd.link.is_some();
        if !movable || !on_name {
            return true;
        }
        let keep = plain && self.selection.contains(index) && self.selection.len() > 1;
        self.dnd.press = Some((self.pointer, keep.then_some(index)));
        !keep
    }

    /// The left button went up: a press kept for a drag was a click after all.
    pub(super) fn dnd_release(&mut self) {
        if let Some((_, Some(index))) = self.dnd.press.take() {
            self.select_only(index);
        }
    }

    /// The pointer moved: a press on the selection far enough becomes a drag.
    pub(super) fn dnd_moved(&mut self) {
        let Some((at, _)) = self.dnd.press else {
            return;
        };
        let (dx, dy) = (self.pointer.x - at.x, self.pointer.y - at.y);
        if dx.hypot(dy) <= drag::THRESHOLD {
            return;
        }
        self.dnd.press = None;
        self.lasso = None;
        let selected: Vec<_> = self
            .selection
            .iter()
            .filter_map(|i| self.entries.get(i))
            .collect();
        if selected.is_empty() {
            return;
        }
        let paths: Vec<PathBuf> = selected.iter().map(|e| e.path.clone()).collect();
        let items = selected
            .iter()
            .take(drag::STACK)
            .map(|e| crate::drag_image::Item {
                glyph: e.icon(),
                tint: match e.kind {
                    Kind::Folder => color::palette().blue_light,
                    _ => color::palette().text_secondary,
                },
                photo: e.is_image().then(|| e.path.clone()),
            })
            .collect();
        tracing::debug!(files = paths.len(), "drag out");
        if self.dnd.send(Command::Drag(paths.clone(), items)) {
            self.dnd.out = Some(paths);
        }
    }

    /// Ctrl held or released during a drop over the window: copy or move.
    pub(super) fn dnd_modifiers(&mut self) {
        if self.dnd.over.is_some() {
            self.dnd.send(Command::Prefer(self.drop_action()));
        }
    }

    fn drop_action(&self) -> Action {
        match self.modifiers.control() {
            true => Action::Copy,
            false => Action::Move,
        }
    }

    /// The selection goes to the clipboard of the session too, for the other applications.
    pub(super) fn dnd_clip(&mut self, mode: Clip, paths: &[PathBuf]) {
        let mode = match mode {
            Clip::Copy => Mode::Copy,
            Clip::Cut => Mode::Cut,
        };
        self.dnd.send(Command::Clip(mode, paths.to_vec()));
    }

    /// Ctrl + V: the files of the session's clipboard, which may come from another application.
    /// `false`: no thread to ask, the clipboard of Evidence is pasted.
    pub(super) fn dnd_paste(&mut self) -> bool {
        matches!(self.location, Location::Folder(_)) && self.dnd.send(Command::Paste)
    }

    /// The folder row under `at` (window coordinates), if any.
    fn folder_at(&self, at: Point) -> Option<usize> {
        let (x, y) = self.in_list(at);
        let top = sherlock_ui::metrics::LIST_TOP_GAP;
        if x < 0.0 || y < top || at.y < self.list_origin().y {
            return None;
        }
        let index = ((y - top) / sherlock_ui::metrics::LIST_ROW_HEIGHT) as usize;
        let entry = self.entries.get(index)?;
        let dragged = self
            .dnd
            .out
            .as_ref()
            .is_some_and(|out| out.contains(&entry.path));
        (entry.kind == Kind::Folder && !dragged).then_some(index)
    }

    /// Where a drop at `target` lands.
    fn drop_dir(&self, target: Option<usize>) -> Option<PathBuf> {
        match target {
            Some(i) => self.entries.get(i).map(|e| e.path.clone()),
            None => match &self.location {
                Location::Folder(dir) => Some(dir.clone()),
                Location::Trash => None,
            },
        }
    }

    pub(super) fn dnd_event(&mut self, event: Event) -> Task<Message> {
        match event {
            Event::Over(x, y) => {
                let at = Point::new(x, y);
                let target = self.folder_at(at);
                // Dropped back in the folder they come from, the files would not move.
                let here = self.drop_dir(None);
                let home = target.is_none()
                    && self
                        .dnd
                        .out
                        .as_ref()
                        .is_some_and(|out| out.iter().all(|p| p.parent() == here.as_deref()));
                let allowed = !home && self.drop_dir(target).is_some();
                let changed = self.dnd.over.as_ref().map(|o| (o.target, o.allowed));
                if changed != Some((target, allowed)) {
                    tracing::debug!(x, y, ?target, allowed, "files dragged over the window");
                }
                if changed != Some((target, allowed)) {
                    let since = match self.dnd.over.as_ref() {
                        Some(o) if o.target == target => o.since,
                        _ => Instant::now(),
                    };
                    if changed.map(|c| c.1) != Some(allowed) {
                        self.dnd.send(Command::Accept(allowed));
                    }
                    if changed.is_none() {
                        self.dnd.send(Command::Prefer(self.drop_action()));
                    }
                    self.dnd.over = Some(Over {
                        target,
                        since,
                        allowed,
                    });
                }
            }
            Event::Left => self.dnd.over = None,
            Event::Dropped(x, y, action, paths) => {
                let target = self.dnd.over.take().and_then(|o| o.target);
                let target = target.or_else(|| self.folder_at(Point::new(x, y)));
                let Some(dest) = self.drop_dir(target) else {
                    return Task::none();
                };
                return self.bring(dest, action, paths);
            }
            Event::DragEnded(taken) => {
                self.dnd.out = None;
                // Moved by another application: the listing shows it.
                if taken {
                    return self.load(&[]);
                }
            }
            Event::Pasted(Some((mode, paths))) => {
                let Some(dest) = self.drop_dir(None) else {
                    return Task::none();
                };
                let action = match mode {
                    Mode::Copy => Action::Copy,
                    Mode::Cut => {
                        // A cut item is pasted only once.
                        self.clipboard = None;
                        Action::Move
                    }
                };
                return self.bring(dest, action, paths);
            }
            Event::Pasted(None) => return self.paste(),
        }
        Task::none()
    }

    /// Copies or moves `paths` into `dest`, off the interface thread.
    fn bring(&mut self, dest: PathBuf, action: Action, paths: Vec<PathBuf>) -> Task<Message> {
        let paths: Vec<PathBuf> = paths
            .into_iter()
            // Moved where they are, nothing happens; a folder never goes into itself.
            .filter(|p| action == Action::Copy || p.parent() != Some(dest.as_path()))
            .filter(|p| !dest.starts_with(p))
            .collect();
        if paths.is_empty() {
            return Task::none();
        }
        match action {
            Action::Copy => self.spawn(texts::copying(paths.len()), move || {
                done(ops::copy_into(&paths, &dest)?)
            }),
            Action::Move => self.spawn(texts::moving(paths.len()), move || {
                done(ops::move_into(&paths, &dest)?)
            }),
        }
    }

    /// The folder under a drop has waited long enough: it opens (spring-loaded folders).
    pub(super) fn dnd_spring(&mut self) -> Task<Message> {
        let Some(over) = self.dnd.over.as_mut() else {
            return Task::none();
        };
        let Some(index) = over
            .target
            .filter(|_| over.since.elapsed() >= drag::SPRING_DELAY)
        else {
            return Task::none();
        };
        over.target = None;
        over.since = Instant::now();
        let Some(dir) = self.entries.get(index).map(|e| e.path.clone()) else {
            return Task::none();
        };
        self.update(Message::Go(Location::Folder(dir)))
    }

    /// A folder lit under a drop: the frames tick for its spring.
    pub(super) fn dnd_waiting(&self) -> bool {
        self.dnd.over.as_ref().is_some_and(|o| o.target.is_some())
    }

    /// How lit the drop target is, between 0 and 1: steady, then blinking before it opens.
    pub(crate) fn drop_light(&self) -> Option<(Option<usize>, f32)> {
        let over = self.dnd.over.as_ref().filter(|o| o.allowed)?;
        let Some(target) = over.target else {
            return Some((None, 1.0));
        };
        let waited = over.since.elapsed().as_secs_f32();
        let delay = drag::SPRING_DELAY.as_secs_f32();
        let blinking = delay * 0.6;
        let light = if waited < blinking {
            1.0
        } else {
            let t = (waited - blinking) / (delay - blinking) * drag::SPRING_BLINKS as f32;
            0.5 + 0.5 * (t * std::f32::consts::TAU).cos()
        };
        Some((Some(target), light))
    }
}
