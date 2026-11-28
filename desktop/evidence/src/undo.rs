//! Undo and redo of what Evidence did to the disk (lot E-10, audit I-10): a rename, a move, a
//! move to the trash, and what it made (a new folder, copies, duplicates). Each step undone
//! gives the step that redoes it, so that the two lists are one mechanism. Nothing is ever
//! deleted for good: undoing a creation puts it in the trash. No iced here.

use std::path::PathBuf;

use crate::ops;
use crate::trash::Trash;

/// Steps kept in each list.
pub const MAX_STEPS: usize = 50;

/// What was done, as it can be undone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// An item renamed from `from` to `to`.
    Renamed { from: PathBuf, to: PathBuf },
    /// Items moved, each from its first path to its second.
    Moved(Vec<(PathBuf, PathBuf)>),
    /// Items that exist now, made by Evidence: undone, they go to the trash.
    Made(Vec<PathBuf>),
    /// Items in the trash, by their ids: undone, they come back to their place.
    Trashed(Vec<String>),
}

impl Step {
    /// Undoes this step, on the disk: the step that does it again. Off the interface thread.
    pub fn undo(&self, trash: Option<&Trash>) -> ops::Result<Self> {
        match self {
            Self::Renamed { from, to } => {
                ops::rename_noreplace(to, from)?;
                Ok(Self::Renamed {
                    from: to.clone(),
                    to: from.clone(),
                })
            }
            Self::Moved(pairs) => {
                let mut back = Vec::new();
                for (from, to) in pairs.iter().rev() {
                    let mut report = ops::Report::default();
                    ops::move_path(to, from, &mut report)?;
                    back.push((to.clone(), from.clone()));
                }
                back.reverse();
                Ok(Self::Moved(back))
            }
            Self::Made(paths) => {
                let trash = trash.ok_or(no_trash())?;
                let ids = paths
                    .iter()
                    .map(|path| trash.put(path))
                    .collect::<ops::Result<Vec<_>>>()?;
                Ok(Self::Trashed(ids))
            }
            Self::Trashed(ids) => {
                let trash = trash.ok_or(no_trash())?;
                let paths = ids
                    .iter()
                    .map(|id| trash.restore(id))
                    .collect::<ops::Result<Vec<_>>>()?;
                Ok(Self::Made(paths))
            }
        }
    }

    /// The items to select once it is done or undone.
    pub fn shown(&self) -> Vec<PathBuf> {
        match self {
            Self::Renamed { to, .. } => vec![to.clone()],
            Self::Moved(pairs) => pairs.iter().map(|(_, to)| to.clone()).collect(),
            Self::Made(paths) => paths.clone(),
            Self::Trashed(_) => Vec::new(),
        }
    }
}

/// Without a trash, nothing made is undone: it would be deleted for good.
fn no_trash() -> ops::Error {
    ops::Error::Io(std::io::ErrorKind::Unsupported.into())
}

/// A move as a step: each source with where it went, the ones that did not move left out.
pub fn moved(sources: &[PathBuf], report: &ops::Report) -> Step {
    Step::Moved(
        sources
            .iter()
            .cloned()
            .zip(report.paths.iter().cloned())
            .filter(|(from, to)| from != to)
            .collect(),
    )
}

/// The undo and redo lists.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct History {
    pub undo: Vec<Step>,
    pub redo: Vec<Step>,
}

impl History {
    /// A new step done by the user: what was undone can no longer be redone.
    pub fn done(&mut self, step: Step) {
        push(&mut self.undo, step);
        self.redo.clear();
    }

    /// A step undone: the step that redoes it.
    pub fn undone(&mut self, step: Step) {
        push(&mut self.redo, step);
    }

    /// A step redone: the step that undoes it again.
    pub fn redone(&mut self, step: Step) {
        push(&mut self.undo, step);
    }
}

fn push(list: &mut Vec<Step>, step: Step) {
    if list.len() == MAX_STEPS {
        list.remove(0);
    }
    list.push(step);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;
    use std::fs;

    #[test]
    fn every_step_undoes_and_redoes() {
        let tmp = TempDir::new("undo");
        let trash = Trash::at(&tmp.path().join("donnees/Trash"));
        let (a, b) = (tmp.path().join("a.txt"), tmp.path().join("b.txt"));
        fs::write(&a, "a").expect("write");

        // Renamed: back, then again.
        fs::rename(&a, &b).expect("rename");
        let renamed = Step::Renamed {
            from: a.clone(),
            to: b.clone(),
        };
        let redo = renamed.undo(Some(&trash)).expect("undo");
        assert!(a.exists() && !b.exists());
        redo.undo(Some(&trash)).expect("redo");
        assert!(b.exists() && !a.exists());

        // Moved into a folder: back out.
        let dir = tmp.path().join("dossier");
        fs::create_dir(&dir).expect("folder");
        let report = ops::move_into(std::slice::from_ref(&b), &dir).expect("move");
        let step = moved(std::slice::from_ref(&b), &report);
        assert_eq!(step.shown(), [dir.join("b.txt")]);
        step.undo(Some(&trash)).expect("undo");
        assert!(b.exists());

        // Made: to the trash, then back.
        let made = Step::Made(vec![b.clone()]);
        let trashed = made.undo(Some(&trash)).expect("undo");
        assert!(!b.exists());
        assert!(matches!(trashed, Step::Trashed(ref ids) if ids.len() == 1));
        let again = trashed.undo(Some(&trash)).expect("redo");
        assert!(b.exists());
        assert_eq!(again, Step::Made(vec![b.clone()]));
        // Without a trash, a creation is never deleted for good.
        assert!(Step::Made(vec![b.clone()]).undo(None).is_err());
        assert!(b.exists());
    }

    #[test]
    fn a_new_step_forgets_what_was_undone_and_the_lists_are_bounded() {
        let mut history = History::default();
        for i in 0..MAX_STEPS + 5 {
            history.done(Step::Made(vec![PathBuf::from(format!("/x/{i}"))]));
        }
        assert_eq!(history.undo.len(), MAX_STEPS);
        assert_eq!(history.undo[0], Step::Made(vec![PathBuf::from("/x/5")]));
        history.undone(Step::Trashed(vec!["1".into()]));
        assert_eq!(history.redo.len(), 1);
        history.done(Step::Trashed(Vec::new()));
        assert!(history.redo.is_empty());
    }
}
