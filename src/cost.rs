//! Costs and the concrete payments they expand to.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::id::{EntityId, ResourceId, ZoneKind};

/// How many of something: a fixed number, or a number the player chooses when
/// paying (then available to the effect as `x`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Count {
    /// A printed number.
    Fixed(u32),
    /// Chosen by the player (`N`); all resources paid for it must be identical.
    Var,
    /// The number of resources in the placement cost of a card bound while
    /// paying (the discarded card, or else the destroyed one), plus a bonus.
    /// Only meaningful as the size of a gain, not inside a cost.
    CostOf {
        /// Use the discarded card (`true`) or the card bound as `card`.
        discarded: bool,
        /// Added to the number of resources.
        bonus:     u32,
    },
}

/// Which card a destroy cost removes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Which {
    /// The card carrying the ability.
    This,
    /// Another card of yours.
    Other,
    /// Any card of yours, possibly this one.
    Own,
}

/// What has to be paid to buy a card or use an ability.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cost {
    /// Pay these resources to the bank.
    Resources(Vec<(ResourceId, u32)>),
    /// Pay any one of the alternatives.
    Or(Vec<Cost>),
    /// Pay all of them.
    All(Vec<Cost>),
    /// Exhaust the card carrying the ability.
    Exhaust,
    /// Pay resources of the player's choice from their pool: any mix of `n`
    /// (not of the excluded kinds), or with `identical` all of one kind.
    Any {
        /// How many.
        n:         Count,
        /// Kinds that may not be paid.
        exclude:   Vec<ResourceId>,
        /// All the same kind.
        identical: bool,
    },
    /// Take resources off the card carrying the ability (they go back to the
    /// bank).
    TakeFromCard {
        /// Resource kind.
        res: ResourceId,
        /// Amount.
        n:   u32,
    },
    /// Exhaust another ready card of yours carrying this tag.
    TurnOther {
        /// Required tag.
        tag: String,
    },
    /// Destroy a card of yours carrying this tag.
    Destroy {
        /// Which card.
        which: Which,
        /// Required tag.
        tag:   String,
    },
    /// Discard cards from your hand.
    Discard {
        /// The zone kind of the hand.
        from: ZoneKind,
        /// How many.
        n:    u32,
    },
}

/// One concrete way of paying a [`Cost`] (no choices left).
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Payment {
    /// Resources to pay to the bank.
    pub resources: BTreeMap<ResourceId, u32>,
    /// Whether the source card gets exhausted.
    pub exhaust:   bool,
    /// Resources taken off the source card (back to the bank).
    pub from_card: BTreeMap<ResourceId, u32>,
    /// Other cards exhausted as part of the cost.
    pub turned:    Vec<EntityId>,
    /// The card destroyed as part of the cost.
    pub destroyed: Option<EntityId>,
    /// Cards discarded from hand as part of the cost.
    pub discarded: Vec<EntityId>,
    /// The number chosen for a variable count.
    pub x:         Option<u32>,
}

impl Payment {
    /// Changes the amount of one resource by `delta`, never going below zero.
    pub fn adjust(&mut self, res: ResourceId, delta: i32) {
        let current = i64::from(self.resources.get(&res).copied().unwrap_or(0));
        let new = (current + i64::from(delta)).max(0);
        if new == 0 {
            self.resources.remove(&res);
        } else {
            self.resources.insert(res, u32::try_from(new).unwrap_or(u32::MAX));
        }
    }

    /// The resource paid when exactly one unit was paid (what "put the paid
    /// resource on a card" refers to).
    pub fn single_paid(&self) -> Option<ResourceId> {
        let mut it = self.resources.iter();
        match (it.next(), it.next()) {
            (Some((res, 1)), None) => Some(*res),
            _ => None,
        }
    }

    /// Total number of resource units paid.
    pub fn total(&self) -> u32 {
        self.resources.values().sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: ResourceId = ResourceId(0);
    const B: ResourceId = ResourceId(1);

    #[test]
    fn adjust_never_goes_below_zero_and_can_add_new_resources() {
        let mut p = Payment::default();
        p.resources.insert(A, 2);
        p.adjust(A, -5);
        assert_eq!(p.resources.len(), 0);
        p.adjust(B, 3);
        assert_eq!(p.resources[&B], 3);
    }

    #[test]
    fn single_paid_only_for_exactly_one_unit() {
        let mut p = Payment::default();
        assert_eq!(p.single_paid(), None);
        p.resources.insert(A, 1);
        assert_eq!(p.single_paid(), Some(A));
        p.resources.insert(B, 1);
        assert_eq!(p.single_paid(), None);
        assert_eq!(p.total(), 2);
    }
}
