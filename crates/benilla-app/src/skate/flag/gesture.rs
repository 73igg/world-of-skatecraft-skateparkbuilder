//! Select is a host control: a hold places once, and releasing a short press returns once.
pub(super) const HOLD_MS: u64 = 700;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Place,
    Return,
}
#[derive(Default)]
pub(crate) struct Gesture {
    pressed_at: Option<u64>,
    placed: bool,
    blocked: bool,
}
impl Gesture {
    pub(crate) fn cancel(&mut self) {
        self.pressed_at = None;
        self.placed = false;
        self.blocked = true;
    }
    pub(crate) fn update(&mut self, now_ms: u64, held: bool, allowed: bool) -> Option<Action> {
        if !allowed {
            self.cancel();
            if !held {
                self.blocked = false;
            }
            return None;
        }
        if self.blocked {
            if !held {
                self.blocked = false;
            }
            return None;
        }
        if held {
            let started = *self.pressed_at.get_or_insert(now_ms);
            if !self.placed && now_ms.saturating_sub(started) >= HOLD_MS {
                self.placed = true;
                return Some(Action::Place);
            }
        } else if let Some(started) = self.pressed_at.take() {
            let placed = std::mem::take(&mut self.placed);
            if !placed {
                return Some(if now_ms.saturating_sub(started) >= HOLD_MS {
                    Action::Place
                } else {
                    Action::Return
                });
            }
        }
        None
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn holds_place_once_and_never_return_on_release() {
        let mut g = Gesture::default();
        assert_eq!(g.update(0, true, true), None);
        assert_eq!(g.update(699, true, true), None);
        assert_eq!(g.update(700, true, true), Some(Action::Place));
        assert_eq!(g.update(1500, true, true), None);
        assert_eq!(g.update(1600, false, true), None);
        assert_eq!(g.update(1800, true, true), None);
        assert_eq!(g.update(1900, false, true), Some(Action::Return));
        assert_eq!(g.update(1910, false, true), None);
    }
    #[test]
    fn mode_changes_and_reconnects_require_a_fresh_press() {
        let mut g = Gesture::default();
        g.update(0, true, true);
        g.update(300, true, false);
        assert_eq!(g.update(800, true, true), None);
        assert_eq!(g.update(900, false, true), None);
        g.update(1000, true, true);
        g.cancel();
        assert_eq!(g.update(1050, false, true), None);
        g.update(1100, true, true);
        assert_eq!(g.update(1200, false, true), Some(Action::Return));
    }
}
