//! The tabbed pane under the editor: Problems, Output and Terminal. This file holds the
//! state and the rules for switching tabs by themselves; `shell.rs` draws the tabs and
//! `terminal_view.rs` draws the terminal.

use crate::terminal::Terminal;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum BottomTab {
    #[default]
    Problems,
    Output,
    Terminal,
}

impl BottomTab {
    pub const ALL: [BottomTab; 3] = [BottomTab::Problems, BottomTab::Output, BottomTab::Terminal];

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }
}

/// How long a click on a tab protects the choice from automatic switching.
pub(crate) const USER_GRACE_SECS: f64 = 2.5;

/// A shell that is not the one the Terminal pane shows. The shown shell lives in `BottomPane`'s own
/// terminal fields (which the rest of the app reads), and is parked here when another is shown.
pub(crate) struct Session {
    pub number: u32,
    pub terminal: Option<Terminal>,
    pub root: Option<std::path::PathBuf>,
    pub error: Option<String>,
    pub view: crate::terminal_view::ViewState,
}

/// Selected tab, the terminal and what the automatic switching needs to know.
#[derive(Default)]
pub(crate) struct BottomPane {
    pub tab: BottomTab,
    /// When (egui time, seconds) the user last clicked a tab.
    last_click: Option<f64>,
    /// The terminal view had keyboard focus in the latest frame.
    pub terminal_focused: bool,
    pub terminal: Option<Terminal>,
    /// The folder the shell was started in (to notice when the project folder moves on).
    pub terminal_root: Option<std::path::PathBuf>,
    /// The shell could not be started.
    pub terminal_error: Option<String>,
    pub view: crate::terminal_view::ViewState,
    /// The terminal was drawn in the previous frame (so a newly shown one asks for focus).
    pub view_shown: bool,
    /// The number of the shell shown (0 before the first one: shell 1).
    pub shown_shell: u32,
    /// The other shells, parked while another is shown.
    pub others: Vec<Session>,
}

impl BottomPane {
    /// The number of the shell shown (`Shell 1` for the first).
    pub fn number_of_shown(&self) -> u32 {
        self.shown_shell.max(1)
    }

    /// The shells in order, with whether each is the one shown.
    pub fn shells(&self) -> Vec<(u32, bool)> {
        let shown = self.number_of_shown();
        let mut all: Vec<(u32, bool)> = self.others.iter().map(|s| (s.number, false)).collect();
        all.push((shown, true));
        all.sort_by_key(|(n, _)| *n);
        all
    }

    /// Opens another shell (numbered after the last) and shows it. It starts when the pane draws.
    pub fn new_shell(&mut self) {
        let number = self.shells().last().map_or(1, |(n, _)| n + 1);
        let parked = self.park();
        self.others.push(parked);
        self.shown_shell = number;
        self.view_shown = false;
    }

    /// Shows shell `number`, parking the one shown. Nothing happens for an unknown number.
    pub fn show_shell(&mut self, number: u32) {
        if number == self.number_of_shown() {
            return;
        }
        let Some(pos) = self.others.iter().position(|s| s.number == number) else {
            return;
        };
        let next = self.others.remove(pos);
        let parked = self.park();
        self.others.push(parked);
        self.unpark(next);
    }

    /// Closes shell `number` (its shell ends). The last shell stays, since the pane always shows one.
    pub fn close_shell(&mut self, number: u32) {
        if self.others.is_empty() {
            return;
        }
        if number == self.number_of_shown() {
            // Another one is shown first; the closed one is dropped with its shell.
            let next = self.others.pop().expect("checked: there are others");
            drop(self.park());
            self.unpark(next);
        } else {
            self.others.retain(|s| s.number != number);
        }
    }

    /// Takes the shown shell out of the pane's fields.
    fn park(&mut self) -> Session {
        Session {
            number: self.number_of_shown(),
            terminal: self.terminal.take(),
            root: self.terminal_root.take(),
            error: self.terminal_error.take(),
            view: std::mem::take(&mut self.view),
        }
    }

    /// Shows `session` in the pane's fields; it asks for keyboard focus when it is drawn.
    fn unpark(&mut self, session: Session) {
        self.shown_shell = session.number;
        self.terminal = session.terminal;
        self.terminal_root = session.root;
        self.terminal_error = session.error;
        self.view = session.view;
        self.view_shown = false;
        self.terminal_focused = false;
    }

    /// The user picked a tab.
    pub fn choose(&mut self, tab: BottomTab, now: f64) {
        self.tab = tab;
        self.last_click = Some(now);
    }

    /// The program wants to show `tab` (a build started or failed...). Returns
    /// whether it switched. It does not when the user chose a tab a moment ago, nor away from the
    /// terminal while the user types in it.
    pub fn auto(&mut self, tab: BottomTab, now: f64) -> bool {
        if self.tab == tab {
            return false;
        }
        if self.last_click.is_some_and(|t| now - t < USER_GRACE_SECS) {
            return false;
        }
        if self.tab == BottomTab::Terminal && self.terminal_focused {
            return false;
        }
        self.tab = tab;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switches_by_itself_when_nobody_touched_the_tabs() {
        let mut b = BottomPane::default();
        assert!(b.auto(BottomTab::Output, 10.0));
        assert_eq!(b.tab, BottomTab::Output);
        assert!(b.auto(BottomTab::Terminal, 10.1));
        assert_eq!(b.tab, BottomTab::Terminal);
        // Already there: nothing to do.
        assert!(!b.auto(BottomTab::Terminal, 10.2));
    }

    #[test]
    fn a_recent_click_wins_over_automatic_switching() {
        let mut b = BottomPane::default();
        b.choose(BottomTab::Output, 100.0);
        assert!(!b.auto(BottomTab::Problems, 100.5));
        assert!(!b.auto(BottomTab::Terminal, 102.4));
        assert_eq!(b.tab, BottomTab::Output);
        // After the grace period it may switch again.
        assert!(b.auto(BottomTab::Problems, 102.6));
        assert_eq!(b.tab, BottomTab::Problems);
    }

    #[test]
    fn typing_in_the_terminal_is_never_interrupted() {
        let mut b = BottomPane::default();
        b.choose(BottomTab::Terminal, 0.0);
        b.terminal_focused = true;
        assert!(!b.auto(BottomTab::Output, 1000.0));
        assert_eq!(b.tab, BottomTab::Terminal);
        // Without focus (the user clicked into the editor) the terminal tab can be left.
        b.terminal_focused = false;
        assert!(b.auto(BottomTab::Output, 1000.0));
        assert_eq!(b.tab, BottomTab::Output);
    }

    #[test]
    fn tab_order_is_stable() {
        let names: Vec<_> = BottomTab::ALL.iter().map(|t| t.index()).collect();
        assert_eq!(names, [0, 1, 2]);
    }
}

#[cfg(test)]
mod shell_tests {
    use super::*;

    fn numbers(pane: &BottomPane) -> Vec<(u32, bool)> {
        pane.shells()
    }

    #[test]
    fn new_shells_are_numbered_after_the_last_and_shown() {
        let mut pane = BottomPane::default();
        assert_eq!(numbers(&pane), [(1, true)], "the first shell");
        pane.new_shell();
        assert_eq!(numbers(&pane), [(1, false), (2, true)]);
        pane.new_shell();
        assert_eq!(numbers(&pane), [(1, false), (2, false), (3, true)]);
        assert!(!pane.view_shown, "a new shell asks for focus when drawn");
    }

    #[test]
    fn a_shell_is_shown_by_its_number_and_the_others_are_kept() {
        let mut pane = BottomPane::default();
        pane.new_shell();
        pane.new_shell();
        pane.show_shell(1);
        assert_eq!(numbers(&pane), [(1, true), (2, false), (3, false)]);
        pane.show_shell(9);
        assert_eq!(
            pane.number_of_shown(),
            1,
            "an unknown number changes nothing"
        );
    }

    #[test]
    fn closing_the_shown_shell_shows_another_and_closing_another_keeps_the_shown_one() {
        let mut pane = BottomPane::default();
        pane.new_shell();
        pane.new_shell();
        pane.close_shell(3);
        assert_eq!(
            numbers(&pane),
            [(1, false), (2, true)],
            "shell 3 was shown: 2 is shown now"
        );
        pane.close_shell(1);
        assert_eq!(numbers(&pane), [(2, true)]);
    }

    #[test]
    fn the_last_shell_cannot_be_closed() {
        let mut pane = BottomPane::default();
        pane.close_shell(1);
        assert_eq!(numbers(&pane), [(1, true)]);
        pane.new_shell();
        pane.close_shell(2);
        pane.close_shell(1);
        assert_eq!(
            numbers(&pane),
            [(1, true)],
            "shell 2 was shown: 1 is shown, and it is the one left"
        );
    }
}
