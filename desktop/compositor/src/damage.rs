//! Damage (lot P-01): the elements the compositor draws itself (borders, shadows, the snap
//! preview, the background discs) keep their identity from one frame to the next. Smithay's
//! damage tracking (`OutputDamageTracker`, inside each `DrmCompositor`) then redraws only what
//! changed: a window that draws again costs its own area, not the whole screen.
//!
//! Smithay's `PixelShaderElement` takes a new `Id` each time it is built: built again every
//! frame, it looked new every frame, and the whole screen was redrawn. See "Damage" in
//! `docs/compositor.md`.

use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
};

use smithay::{
    backend::renderer::{
        Renderer,
        element::{Element, Id, Kind, RenderElement, UnderlyingStorage},
        utils::{CommitCounter, DamageSet, OpaqueRegions},
    },
    reexports::wayland_server::backend::ObjectId,
    utils::{Buffer, Physical, Point, Rectangle, Scale, Transform},
};

/// What an element drawn by the compositor is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Part {
    Border,
    Shadow,
    Preview,
    /// A window during its minimize effect (lot D-19).
    Genie,
    /// The last image of a window while it closes (lot D-34).
    Closing,
    /// One of the background discs, by its index.
    Disc(usize),
    /// The title bar the compositor draws above a window (lot D-08).
    TitleBar,
    /// Mission Control (lot D-33): the dim over the wallpaper, the accent border of the window
    /// under the pointer, the title under a window, the title bar of a window in a thumbnail.
    Dim,
    Highlight,
    Label,
    /// The « × » of the window under the pointer (lot D-33c).
    CloseButton,
    MiniBar,
    /// Mission Control: the background, the border and the name of the thumbnail of a
    /// workspace, by its index.
    Thumbnail(usize),
    ThumbnailBorder(usize),
    ThumbnailLabel(usize),
    /// The on-screen display of the media keys (lot D-35).
    Osd,
    /// The screenshot tool (lots D-35c, D-35e): its shapes (dim, area, handles, toolbar,
    /// buttons) and its texts, by their order.
    CaptureShape(usize),
    CaptureLabel(usize),
    /// The ring of a click shown (lot C-01), by its number.
    Click(u64),
    /// The background of a locked screen, and the pills of the rescue screen, by their index
    /// (lot D-25a).
    LockBackground,
    LockLabel(usize),
}

/// Identity of an element drawn by the compositor: its part, and the window it belongs to, by
/// its main surface.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Key {
    pub owner: Option<ObjectId>,
    pub part: Part,
}

impl Key {
    pub fn of(owner: Option<ObjectId>, part: Part) -> Self {
        Self { owner, part }
    }
}

struct Slot {
    id: Id,
    commit: CommitCounter,
    params: u64,
    used: bool,
}

/// Identities of the elements of one screen. A screen has its own damage tracker: each one
/// keeps its own `Ids`.
#[derive(Default)]
pub struct Ids {
    slots: HashMap<Key, Slot>,
}

impl Ids {
    /// `inner` under the identity of `key`. Its commit changes when `params` change: a hash of
    /// what it draws besides its place, size and opacity, which the damage tracker compares by
    /// itself.
    pub fn stamp<E>(&mut self, key: Key, params: u64, inner: E) -> Stamped<E> {
        let slot = self.slots.entry(key).or_insert_with(|| Slot {
            id: Id::new(),
            commit: CommitCounter::default(),
            params,
            used: false,
        });
        if slot.params != params {
            slot.params = params;
            slot.commit.increment();
        }
        slot.used = true;
        Stamped {
            inner,
            id: slot.id.clone(),
            commit: slot.commit,
        }
    }

    /// End of a frame: the identities it did not use are forgotten.
    pub fn sweep(&mut self) {
        self.slots.retain(|_, slot| std::mem::take(&mut slot.used));
    }

    /// Forgets every identity: the next frame is new from end to end. The bench uses it to
    /// reproduce the rendering from before lot P-01.
    pub fn clear(&mut self) {
        self.slots.clear();
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
}

/// Hash of the parameters of an element, for `Ids::stamp`.
pub fn params_hash(values: &[f32]) -> u64 {
    let mut hasher = DefaultHasher::new();
    for value in values {
        value.to_bits().hash(&mut hasher);
    }
    hasher.finish()
}

/// An element under a stable identity: damaged whole when its parameters change, not at all
/// otherwise. Moves, resizes and changes of opacity are seen by the damage tracker.
pub struct Stamped<E> {
    inner: E,
    id: Id,
    commit: CommitCounter,
}

impl<E: Element> Element for Stamped<E> {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.commit
    }

    fn location(&self, scale: Scale<f64>) -> Point<i32, Physical> {
        self.inner.location(scale)
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        self.inner.src()
    }

    fn transform(&self) -> Transform {
        self.inner.transform()
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.inner.geometry(scale)
    }

    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        if commit == Some(self.commit) {
            return DamageSet::default();
        }
        DamageSet::from_slice(&[Rectangle::from_size(self.geometry(scale).size)])
    }

    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        self.inner.opaque_regions(scale)
    }

    fn alpha(&self) -> f32 {
        self.inner.alpha()
    }

    fn kind(&self) -> Kind {
        self.inner.kind()
    }
}

impl<R: Renderer, E: RenderElement<R>> RenderElement<R> for Stamped<E> {
    fn draw(
        &self,
        frame: &mut R::Frame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), R::Error> {
        self.inner.draw(frame, src, dst, damage, opaque_regions)
    }

    fn underlying_storage(&self, renderer: &mut R) -> Option<UnderlyingStorage<'_>> {
        self.inner.underlying_storage(renderer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_element_keeps_its_identity_while_it_is_drawn() {
        let mut ids = Ids::default();
        let key = Key::of(None, Part::Preview);
        let first = ids.stamp(key.clone(), params_hash(&[1.0]), ());
        ids.sweep();
        // Same parameters: same identity, same commit, nothing to redraw.
        let same = ids.stamp(key.clone(), params_hash(&[1.0]), ());
        assert_eq!(same.id, first.id);
        assert_eq!(same.commit, first.commit);
        ids.sweep();
        // Other parameters: same identity, a new commit.
        let changed = ids.stamp(key.clone(), params_hash(&[2.0]), ());
        assert_eq!(changed.id, first.id);
        assert_ne!(changed.commit, first.commit);
        ids.sweep();
        // A frame without it: forgotten, and new when it comes back.
        ids.sweep();
        assert!(ids.is_empty());
        let back = ids.stamp(key, params_hash(&[2.0]), ());
        assert_ne!(back.id, first.id);
    }

    #[test]
    fn each_part_has_its_own_identity() {
        let mut ids = Ids::default();
        let border = ids.stamp(Key::of(None, Part::Border), 0, ());
        let shadow = ids.stamp(Key::of(None, Part::Shadow), 0, ());
        let disc = ids.stamp(Key::of(None, Part::Disc(0)), 0, ());
        assert_ne!(border.id, shadow.id);
        assert_ne!(shadow.id, disc.id);
        assert_eq!(ids.len(), 3);
    }
}
