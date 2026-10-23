//! Where the view has been, for going back and forward the way a browser does. Each
//! entry is the whole view as it was when it was left: the journey and its outcome,
//! the cursor, how much of it showed, and the spotlight, so going back puts all of it
//! back rather than only the node it was on.

use std::collections::HashSet;

use codeviz_core::SpanId;
use codeviz_ir::UnitId;

use crate::{disclosure::Disclose, explorer::Side};

/// The most places remembered; older ones are forgotten.
const LIMIT: usize = 200;

/// Everything that says what the view is looking at.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct View {
    /// The active journey.
    pub(crate) track: usize,
    pub(crate) root: Option<UnitId>,
    pub(crate) scenario: Option<usize>,
    pub(crate) cursor: Option<SpanId>,
    pub(crate) disclose: Disclose,
    pub(crate) expanded: HashSet<SpanId>,
    /// The side in the spotlight, when it came on, and the node it was on for.
    pub(crate) spotlight: Option<(Side, f32, usize)>,
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
    /// Records leaving `from` for somewhere new; what was ahead is dropped, as a
    /// browser drops it when you follow a link after going back.
    pub(crate) fn leave(&mut self, from: T) {
        if self.back.last() != Some(&from) {
            self.back.push(from);
        }
        if self.back.len() > LIMIT {
            self.back.remove(0);
        }
        self.forward.clear();
    }

    /// The place before `current`, which becomes the place ahead.
    pub(crate) fn back(&mut self, current: T) -> Option<T> {
        let to = self.back.pop()?;
        self.forward.push(current);
        Some(to)
    }

    /// The place ahead of `current`, which becomes the place before.
    pub(crate) fn forward(&mut self, current: T) -> Option<T> {
        let to = self.forward.pop()?;
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
}
