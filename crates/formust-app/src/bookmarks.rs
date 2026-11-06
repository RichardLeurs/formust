//! Numbered bookmarks (0-9): each number marks one line of one file. They live for the session only;
//! nothing is saved. A bookmark is a line number, so it does not move when lines are added above it.

use crate::files;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The ten bookmark slots, 0 to 9. Each holds a file and a 1-based line.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Bookmarks {
    slots: [Option<(PathBuf, u32)>; 10],
}

impl Bookmarks {
    /// Sets bookmark `n` to `line` of `path`. Setting it again at the same place clears it. Returns
    /// whether the bookmark is now set.
    pub fn toggle(&mut self, n: usize, path: &Path, line: u32) -> bool {
        let Some(slot) = self.slots.get_mut(n) else {
            return false;
        };
        if slot.as_ref().is_some_and(|(p, l)| p == path && *l == line) {
            *slot = None;
            return false;
        }
        *slot = Some((path.to_path_buf(), line));
        true
    }

    /// The lowest free number, when one is free.
    pub fn next_free(&self) -> Option<usize> {
        self.slots.iter().position(Option::is_none)
    }

    /// The bookmark at `path` and `line`, if any.
    pub fn at(&self, path: &Path, line: u32) -> Option<usize> {
        self.slots
            .iter()
            .position(|s| s.as_ref().is_some_and(|(p, l)| p == path && *l == line))
    }

    /// Clears the bookmark at `path` and `line`, if there is one. Returns its number.
    pub fn remove_at(&mut self, path: &Path, line: u32) -> Option<usize> {
        let n = self.at(path, line)?;
        self.slots[n] = None;
        Some(n)
    }

    /// The bookmark after (or before) `path` and `line` in file and line order, wrapping around at the
    /// ends. Returns its number, file and line. A bookmark exactly at `path` and `line` is skipped.
    pub fn step(&self, path: &Path, line: u32, forward: bool) -> Option<(usize, &Path, u32)> {
        let mut all: Vec<(usize, &Path, u32)> = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(n, s)| s.as_ref().map(|(p, l)| (n, p.as_path(), *l)))
            .collect();
        all.sort_by(|a, b| (a.1, a.2).cmp(&(b.1, b.2)));
        let here = (path, line);
        if forward {
            all.iter()
                .copied()
                .find(|(_, p, l)| (*p, *l) > here)
                .or_else(|| all.first().copied())
        } else {
            all.iter()
                .rev()
                .copied()
                .find(|(_, p, l)| (*p, *l) < here)
                .or_else(|| all.last().copied())
        }
    }

    /// Bookmark `n`, when it is set.
    pub fn get(&self, n: usize) -> Option<(&Path, u32)> {
        self.slots.get(n)?.as_ref().map(|(p, l)| (p.as_path(), *l))
    }

    /// The bookmarks of `path`: line -> bookmark number (for the gutter).
    pub fn lines_in(&self, path: &Path) -> BTreeMap<u32, usize> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(n, slot)| match slot {
                Some((p, l)) if p == path => Some((*l, n)),
                _ => None,
            })
            .collect()
    }

    /// Points bookmarks at a moved file or folder to its new place.
    pub fn follow_move(&mut self, from: &Path, to: &Path) {
        for (p, _) in self.slots.iter_mut().flatten() {
            if let Some(moved) = files::moved(p, from, to) {
                *p = moved;
            }
        }
    }

    /// Clears the bookmarks in a deleted file or folder.
    pub fn remove_under(&mut self, path: &Path) {
        for slot in &mut self.slots {
            if slot.as_ref().is_some_and(|(p, _)| p.starts_with(path)) {
                *slot = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bookmark_is_set_moved_and_cleared_by_its_number() {
        let mut b = Bookmarks::default();
        let a = Path::new("/p/a.rs");
        let c = Path::new("/p/c.rs");
        assert!(b.toggle(3, a, 12), "set");
        assert_eq!(b.get(3), Some((a, 12)));
        assert!(!b.toggle(3, a, 12), "the same place clears it");
        assert_eq!(b.get(3), None);
        assert!(b.toggle(3, a, 12));
        assert!(b.toggle(3, c, 4), "setting it elsewhere moves it");
        assert_eq!(b.get(3), Some((c, 4)));
        assert!(b.lines_in(a).is_empty());
        assert_eq!(b.lines_in(c).get(&4), Some(&3));
        assert!(!b.toggle(10, a, 1), "only 0 to 9");
    }

    #[test]
    fn set_next_and_step_go_through_the_bookmarks_in_order_and_wrap() {
        let mut b = Bookmarks::default();
        let a = Path::new("/p/a.rs");
        let c = Path::new("/p/c.rs");
        assert_eq!(b.next_free(), Some(0));
        b.toggle(0, c, 2);
        b.toggle(1, a, 9);
        b.toggle(2, a, 3);
        assert_eq!(b.next_free(), Some(3));
        assert_eq!(b.at(a, 3), Some(2));
        assert_eq!(b.at(a, 4), None);
        // Forward from a.rs:3 (bookmark 2) goes to a.rs:9 (bookmark 1), then wraps to c.rs:2.
        assert_eq!(b.step(a, 3, true), Some((1, a, 9)));
        assert_eq!(b.step(a, 9, true), Some((0, c, 2)));
        assert_eq!(b.step(c, 2, true), Some((2, a, 3)), "wraps to the first");
        // Backward from a.rs:3 wraps to the last, c.rs:2; from a.rs:9 it is a.rs:3.
        assert_eq!(b.step(a, 3, false), Some((0, c, 2)));
        assert_eq!(b.step(a, 9, false), Some((2, a, 3)));
        // Between two bookmarks, the step lands on the neighbour, not the one at the caret.
        assert_eq!(b.step(a, 5, true), Some((1, a, 9)));
        assert_eq!(b.step(a, 5, false), Some((2, a, 3)));
        assert_eq!(Bookmarks::default().step(a, 1, true), None, "none set");
        for n in 0..10 {
            b.toggle(n, a, 100 + n as u32);
        }
        assert_eq!(b.next_free(), None, "all ten are used");
        assert_eq!(
            b.remove_at(a, 101),
            Some(1),
            "unset clears the bookmark on that line"
        );
        assert_eq!(b.remove_at(a, 101), None, "and there is nothing left there");
        assert_eq!(b.next_free(), Some(1));
    }

    #[test]
    fn follow_move_and_remove_under_track_the_files() {
        let mut b = Bookmarks::default();
        b.toggle(1, Path::new("/p/sub/a.rs"), 5);
        b.toggle(2, Path::new("/p/keep.rs"), 6);
        b.follow_move(Path::new("/p/sub"), Path::new("/p/new"));
        assert_eq!(b.get(1), Some((Path::new("/p/new/a.rs"), 5)));
        assert_eq!(b.get(2), Some((Path::new("/p/keep.rs"), 6)));
        // A file moved to a folder of its own name keeps no trailing slash.
        b.follow_move(Path::new("/p/keep.rs"), Path::new("/p/kept.rs"));
        assert_eq!(b.get(2).unwrap().0.to_string_lossy(), "/p/kept.rs");
        b.remove_under(Path::new("/p/new"));
        assert_eq!(b.get(1), None);
        assert_eq!(b.get(2).map(|(_, l)| l), Some(6));
        assert_eq!(b.remove_at(Path::new("/p/kept.rs"), 6), Some(2));
        assert_eq!(b, Bookmarks::default());
    }
}
