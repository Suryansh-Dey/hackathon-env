//! Generic finite state machine helper.

use crate::DomainError;
use std::fmt::Debug;

/// A status enum with an explicit transition table.
pub trait StateMachine: Copy + Debug + PartialEq + Sized {
    /// Returns true when `self → next` is allowed.
    fn can_transition_to(self, next: Self) -> bool;

    /// True for states with no outgoing transitions.
    fn is_terminal(self) -> bool;

    /// Validates a transition and returns the next state.
    fn transition(self, next: Self) -> Result<Self, DomainError> {
        if self.can_transition_to(next) {
            Ok(next)
        } else {
            Err(DomainError::InvalidTransition { from: format!("{self:?}"), to: format!("{next:?}") })
        }
    }
}
