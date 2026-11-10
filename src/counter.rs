//! Generic bounded counters, used for scores and tracks.

use serde::{Deserialize, Serialize};

use crate::{id::CounterId, zone::Owner};

/// A bounded integer value owned by a player or shared.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counter {
    /// Which counter this is.
    pub id:    CounterId,
    /// Who owns it.
    pub owner: Owner,
    /// Current value, always within `min..=max`.
    pub value: i64,
    /// Lower bound.
    pub min:   i64,
    /// Upper bound.
    pub max:   i64,
}

impl Counter {
    /// Creates a counter, clamping the initial value into the bounds.
    pub fn new(id: CounterId, owner: Owner, value: i64, min: i64, max: i64) -> Self {
        Self {
            id,
            owner,
            value: value.clamp(min, max),
            min,
            max,
        }
    }

    /// Adds `delta`, clamping to the bounds. Returns the change that was really
    /// applied.
    pub fn add(&mut self, delta: i64) -> i64 {
        let new = self.value.saturating_add(delta).clamp(self.min, self.max);
        let applied = new - self.value;
        self.value = new;
        applied
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::PlayerId;

    #[test]
    fn add_clamps_and_reports_applied_delta() {
        let mut c = Counter::new(CounterId(0), Owner::Player(PlayerId(0)), 8, 0, 10);
        assert_eq!(c.add(5), 2);
        assert_eq!(c.value, 10);
        assert_eq!(c.add(-100), -10);
        assert_eq!(c.value, 0);
    }

    #[test]
    fn new_clamps_initial_value() {
        assert_eq!(
            Counter::new(CounterId(0), Owner::Shared, 50, 0, 10).value,
            10
        );
    }
}
