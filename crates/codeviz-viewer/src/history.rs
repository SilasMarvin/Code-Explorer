//! Where the view has been, for going back and forward the way a browser does. Each
//! entry is the whole view as it was when it was left: the journey, the cursor, how
//! much of it showed, the function clicked, and where the camera stood, so going back
//! puts all of it back, seen as it was, rather than only the node it was on.
//!
//! One rule says what makes an entry: whatever changes where the journey starts, opens
//! or leaves a journey, selects a function browsing, sets one aside or lets it go, or
//! moves the cursor, by a click, a bar, Enter or each step with the arrows or
//! brackets, so Back retraces a walk a step at a time; a caller of the start set
//! aside along the row above its bar is no place of its own. Back crosses between browsing and a journey as it crosses
//! anything, and never to the place it is at.

use std::collections::HashSet;

use codeviz_core::SpanId;
use codeviz_ir::UnitId;

use crate::{camera::Pose, disclosure::Disclose, explorer::ViewMode};

/// The most places remembered; older ones are forgotten.
const LIMIT: usize = 200;

/// Everything that says what the view is looking at.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct View {
    /// Where the journey starts, browsing too: Enter on it opens that journey again.
    pub(crate) start: Option<UnitId>,
    pub(crate) cursor: Option<SpanId>,
    pub(crate) disclose: Disclose,
    pub(crate) expanded: HashSet<SpanId>,
    /// In a journey, the function set aside.
    pub(crate) aside: Option<usize>,
    /// Whether a journey was open, or the view browsed.
    pub(crate) mode: ViewMode,
    /// Browsing, the function selected.
    pub(crate) selection: Option<usize>,
    /// In a journey, whether its outside callers showed.
    pub(crate) outside: bool,
    /// Where the camera stood.
    pub(crate) pose: Seen,
}

/// Where the camera stood at a place in the history, the module expanded then, and
/// where the camera stood before that module, or any, was expanded: put back with it,
/// but never what makes two places differ, so moving the camera is no move of its own.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Seen {
    pub(crate) pose: Option<Pose>,
    pub(crate) expanded: Option<usize>,
    pub(crate) resting: Option<Pose>,
}

impl PartialEq for Seen {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

/// Places left behind, and places gone back from.
#[derive(Debug)]
pub(crate) struct History<T> {
    back: Vec<T>,
    forward: Vec<T>,
}

impl<T> Default for History<T> {
    fn default() -> Self {
        Self {
            back: Vec::new(),
            forward: Vec::new(),
        }
    }
}

impl<T: PartialEq> History<T> {
    /// Forgets the place last left, as when a move is taken back at once.
    pub(crate) fn forget_last(&mut self) {
        self.back.pop();
    }

    /// Records leaving `from` for somewhere new; what was ahead is dropped, as a
    /// browser drops it when you follow a link after going back. Leaving the place
    /// last left again remembers it as it was seen this time.
    pub(crate) fn leave(&mut self, from: T) {
        match self.back.last_mut() {
            Some(last) if *last == from => *last = from,
            _ => self.back.push(from),
        }
        if self.back.len() > LIMIT {
            self.back.remove(0);
        }
        self.forward.clear();
    }

    /// The place before `current`, which becomes the place ahead: one that differs
    /// from it, so Back never looks like a key that did nothing.
    pub(crate) fn back(&mut self, current: T) -> Option<T> {
        let to = pop_other(&mut self.back, &current)?;
        self.forward.push(current);
        Some(to)
    }

    /// The place ahead of `current`, which becomes the place before.
    pub(crate) fn forward(&mut self, current: T) -> Option<T> {
        let to = pop_other(&mut self.forward, &current)?;
        self.back.push(current);
        Some(to)
    }

    pub(crate) fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub(crate) fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    /// Forgets everything, as when the view first opens.
    pub(crate) fn clear(&mut self) {
        self.back.clear();
        self.forward.clear();
    }
}

/// The last of `places` that is not `current`, dropping those that are.
fn pop_other<T: PartialEq>(places: &mut Vec<T>, current: &T) -> Option<T> {
    while places.last() == Some(current) {
        places.pop();
    }
    places.pop()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goes_back_and_forward_like_a_browser() {
        let mut history = History::default();
        history.leave("a");
        history.leave("b");
        assert_eq!(history.back("c"), Some("b"));
        assert_eq!(history.back("b"), Some("a"));
        assert_eq!(history.back("a"), None, "nothing before the first");
        assert!(history.can_go_forward());
        assert_eq!(history.forward("a"), Some("b"));
        assert_eq!(history.forward("b"), Some("c"));
        assert_eq!(history.forward("c"), None);

        history.back("c");
        history.leave("b");
        assert!(
            !history.can_go_forward(),
            "going somewhere new drops what was ahead"
        );
    }

    #[test]
    fn leaving_the_same_place_twice_remembers_it_once() {
        let mut history = History::default();
        history.leave(1);
        history.leave(1);
        assert_eq!(history.back(2), Some(1));
        assert!(!history.can_go_back());
    }

    #[test]
    fn back_passes_over_the_place_it_is_at() {
        let mut history = History::default();
        history.leave(1);
        history.leave(2);
        assert_eq!(history.back(2), Some(1), "not 2 again");
        assert_eq!(history.forward(1), Some(2));
    }
}
