//! The closing of the shell's menus and panels (lot UI-06): the `xdg_popup`s of anchored
//! surfaces, the bar's and the dock's. They fold back by themselves on Escape, on their own
//! button or on an entry chosen; but when the compositor closes them (a click elsewhere, a
//! workspace switch, Mission Control, the lock) or when their client drops one to open another,
//! the popup is gone at once: iced_exwlshell destroys it on `popup_done`.
//!
//! So the compositor keeps their last image, as it does for a window that closes
//! (`window_anim`): drawn into a texture by the backend (`Snapshotter`) at the moment it is
//! dismissed or destroyed, while its buffer is still there, then shrunk to
//! `motion::POPUP_CLOSE_SCALE` and faded out over `motion::POPUP_CLOSE`, in front of its
//! anchored surface. The ghost takes no input: the click that closed the menu reaches what is
//! under it. A popup that already faded out by itself leaves a transparent image: nothing shows.
//!
//! The menus of applications are not concerned: they go at once (see "Opening and closing" in
//! `docs/compositor.md`).

use std::time::Duration;

use sherlock_ui::motion;
use smithay::{
    backend::renderer::gles::{GlesRenderer, GlesTexture},
    desktop::{PopupKind, PopupManager, find_popup_root_surface},
    output::Output,
    reexports::wayland_server::{
        Resource as _, backend::ObjectId, protocol::wl_surface::WlSurface,
    },
    utils::{Logical, Point, Rectangle, Size},
    wayland::compositor::with_states,
};

use crate::{
    State, anim,
    damage::{Ids, Key, Part},
    genie,
    output::View,
    render::{Item, Scene, SceneElement},
    window_anim,
};

/// Draws a surface tree, without its menus, into a texture of `size` at `scale`: given by the
/// backend, which has the renderer.
pub type Snapshotter =
    Box<dyn FnMut(&State, &WlSurface, Size<i32, Logical>, f64) -> Option<GlesTexture>>;

/// The last image of a shell popup, while it closes.
struct Ghost {
    owner: ObjectId,
    /// In the space.
    rect: Rectangle<i32, Logical>,
    /// `None` without a renderer (the tests): nothing is drawn, the closing still runs.
    texture: Option<GlesTexture>,
    fade: motion::Fade,
}

/// The shell popups closing, and the backend's way to keep their last image.
#[derive(Default)]
pub struct PopupAnims {
    ghosts: Vec<Ghost>,
    snapshotter: Option<Snapshotter>,
}

impl State {
    /// Plugs in the backend's way to draw a popup into a texture.
    pub fn set_popup_snapshotter(&mut self, snapshotter: Snapshotter) {
        self.popup_anims.snapshotter = Some(snapshotter);
    }

    /// Shell popups whose closing is drawn.
    pub fn closing_popups(&self) -> usize {
        self.popup_anims.ghosts.len()
    }

    /// The popups of the anchored surface `root` are dismissed by the compositor: each keeps
    /// its last image while it closes. Call it before `PopupManager::dismiss_popup`.
    pub(crate) fn ghost_popups_of(&mut self, root: &WlSurface) {
        let popups: Vec<PopupKind> = PopupManager::popups_for_surface(root)
            .map(|(popup, _)| popup)
            .collect();
        for popup in popups {
            self.ghost_popup(&popup);
        }
    }

    /// `popup` goes away: keeps its last image while it closes, when it belongs to an anchored
    /// surface and shows something. Once per popup: dismissed, then destroyed by its client,
    /// it closes once.
    pub(crate) fn ghost_popup(&mut self, popup: &PopupKind) {
        if !self.window_animations() || self.is_locked() {
            return;
        }
        let surface = popup.wl_surface().clone();
        let owner = surface.id();
        if self.popup_anims.ghosts.iter().any(|g| g.owner == owner) {
            return;
        }
        let Ok(root) = find_popup_root_surface(popup) else {
            return;
        };
        // Anchored surfaces only: the bar, the dock.
        let Some((output, layer)) = self.layer_for(&root) else {
            return;
        };
        let Some(offset) = offset_in_root(&surface) else {
            return;
        };
        let size = smithay::desktop::utils::bbox_from_surface_tree(&surface, (0, 0)).size;
        if size.is_empty() || !has_content(&surface) {
            return;
        }
        let loc: Point<i32, Logical> = layer + offset - popup.geometry().loc;
        let scale = output.current_scale().fractional_scale();
        let mut snapshotter = self.popup_anims.snapshotter.take();
        let texture = snapshotter
            .as_mut()
            .and_then(|draw| draw(self, &surface, size, scale));
        self.popup_anims.snapshotter = snapshotter;
        let now = self.now();
        let fade = motion::Fade::eased(
            1.0,
            0.0,
            now,
            motion::POPUP_CLOSE,
            motion::WINDOW_CLOSE_EASE,
        );
        tracing::debug!(?owner, "shell popup closing");
        self.popup_anims.ghosts.push(Ghost {
            owner,
            rect: Rectangle::new(loc, size),
            texture,
            fade,
        });
        self.queue_redraw();
    }

    /// Frees the popup closings over at this time. Called after each loop iteration.
    pub(crate) fn end_popup_closings(&mut self) {
        let now = self.now();
        self.popup_anims.ghosts.retain(|g| !g.fade.is_done(now));
    }
}

/// Where the popup `surface` sits from the surface its chain of menus starts from: the sum of
/// the places each menu was given in its parent, as smithay draws them.
fn offset_in_root(surface: &WlSurface) -> Option<Point<i32, Logical>> {
    use smithay::wayland::{
        compositor::get_role,
        shell::xdg::{XDG_POPUP_ROLE, XdgPopupSurfaceData},
    };
    let mut offset = Point::default();
    let mut current = surface.clone();
    while get_role(&current) == Some(XDG_POPUP_ROLE) {
        let (loc, parent) = with_states(&current, |states| {
            let data = states.data_map.get::<XdgPopupSurfaceData>()?.lock().ok()?;
            Some((data.current.geometry.loc, data.parent.clone()?))
        })?;
        offset += loc;
        current = parent;
    }
    Some(offset)
}

/// Whether the surface has a buffer: a popup unmapped by its client leaves nothing to keep.
fn has_content(surface: &WlSurface) -> bool {
    use smithay::backend::renderer::utils::RendererSurfaceStateUserData;
    with_states(surface, |states| {
        let data = states.data_map.get::<RendererSurfaceStateUserData>();
        data.and_then(|d| d.lock().ok().map(|d| d.buffer().is_some()))
            .unwrap_or(false)
    })
}

/// Items of the shell popups closing at `now`, seen on `output` through `view`, the last one
/// in front, and whether one runs. They go in front of the anchored surfaces.
pub(crate) fn push_closing(
    items: &mut Vec<Item>,
    state: &State,
    (_output, view): (&Output, &View),
    now: Duration,
) -> bool {
    let mut running = false;
    for ghost in state.popup_anims.ghosts.iter().rev() {
        if ghost.fade.is_done(now) {
            continue;
        }
        running = true;
        let Some(texture) = &ghost.texture else {
            continue;
        };
        let (scale, alpha) = motion::popup_closing(ghost.fade.value(now));
        let rect = view.rect(anim::scaled(ghost.rect.to_f64(), scale));
        if !rect.overlaps(view.frame) {
            continue;
        }
        items.push(Item::Snapshot {
            key: Key::of(Some(ghost.owner.clone()), Part::Closing),
            texture: texture.clone(),
            rect,
            alpha,
        });
    }
    running
}

/// `surface` as it shows, into a texture of `size` at `scale`. The backends call it from their
/// `Snapshotter`.
pub fn snapshot(
    scene: &Scene,
    renderer: &mut GlesRenderer,
    state: &State,
    surface: &WlSurface,
    size: Size<i32, Logical>,
    scale: f64,
) -> Option<GlesTexture> {
    let pixels = size.to_f64().to_physical(scale).to_i32_round();
    let view = genie::offscreen(Rectangle::from_size(size), pixels, scale);
    let item = Item::Surface {
        surface: surface.clone(),
        origin: (0, 0).into(),
        scale,
        alpha: 1.0,
    };
    let mut ids = Ids::default();
    let mode = state.wallpaper.mode;
    let elements: Vec<SceneElement> = scene
        .elements(renderer, &mut ids, item, &view, &[], mode)
        .into_iter()
        .collect();
    window_anim::draw(renderer, pixels, &elements, [0.0; 4])
        .inspect_err(|err| tracing::warn!("closing: drawing the popup: {err}"))
        .ok()
}
