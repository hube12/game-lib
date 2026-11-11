//! Events: everything that happens is reported as an event, and abilities
//! trigger on them.

use serde::{Deserialize, Serialize};

use crate::{
    id::{CounterId, EntityId, PhaseId, PlayerId, ResourceId, ZoneId},
    modifier::Subject,
    zone::Owner,
};

/// Something that happened in the game.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Event {
    /// A new round started (rounds count from 1).
    RoundStarted(u32),
    /// A round ended.
    RoundEnded(u32),
    /// A phase started.
    PhaseStarted(PhaseId),
    /// A player's turn started.
    TurnStarted(PlayerId),
    /// A player's turn ended.
    TurnEnded(PlayerId),
    /// A player passed.
    Passed(PlayerId),
    /// An entity changed zone.
    EntityMoved {
        /// The entity.
        entity: EntityId,
        /// Where it was.
        from:   ZoneId,
        /// Where it is now.
        to:     ZoneId,
    },
    /// A zone was shuffled.
    Shuffled(ZoneId),
    /// A player bought a card from a market.
    CardBought {
        /// The buyer.
        player: PlayerId,
        /// The card.
        entity: EntityId,
    },
    /// A player activated an ability.
    AbilityActivated {
        /// The player.
        player:  PlayerId,
        /// The card carrying the ability.
        entity:  EntityId,
        /// Index of the ability on the card.
        ability: usize,
    },
    /// Resources moved from the bank to a player or card.
    ResourceGained {
        /// Receiver.
        to:  Subject,
        /// Resource type.
        res: ResourceId,
        /// Amount.
        n:   u32,
    },
    /// A player paid resources to the bank.
    ResourcePaid {
        /// The payer.
        player: PlayerId,
        /// Resource type.
        res:    ResourceId,
        /// Amount.
        n:      u32,
    },
    /// A counter changed.
    CounterChanged {
        /// The counter.
        counter: CounterId,
        /// Its owner.
        owner:   Owner,
        /// The change really applied.
        delta:   i64,
    },
    /// The engine waits for these players to decide.
    DecisionRequested(Vec<PlayerId>),
    /// A player answered a pending decision.
    DecisionMade(PlayerId),
    /// The game is over.
    GameEnded(Vec<PlayerId>),
}

/// The kind of an [`Event`] without its payload; what abilities and modifiers
/// match on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventKind {
    /// [`Event::RoundStarted`]
    RoundStarted,
    /// [`Event::RoundEnded`]
    RoundEnded,
    /// [`Event::PhaseStarted`]
    PhaseStarted,
    /// [`Event::TurnStarted`]
    TurnStarted,
    /// [`Event::TurnEnded`]
    TurnEnded,
    /// [`Event::Passed`]
    Passed,
    /// [`Event::EntityMoved`]
    EntityMoved,
    /// [`Event::Shuffled`]
    Shuffled,
    /// [`Event::CardBought`]
    CardBought,
    /// [`Event::AbilityActivated`]
    AbilityActivated,
    /// [`Event::ResourceGained`]
    ResourceGained,
    /// [`Event::ResourcePaid`]
    ResourcePaid,
    /// [`Event::CounterChanged`]
    CounterChanged,
    /// [`Event::DecisionRequested`]
    DecisionRequested,
    /// [`Event::DecisionMade`]
    DecisionMade,
    /// [`Event::GameEnded`]
    GameEnded,
}

impl Event {
    /// The kind of this event.
    pub const fn kind(&self) -> EventKind {
        match self {
            Self::RoundStarted(_) => EventKind::RoundStarted,
            Self::RoundEnded(_) => EventKind::RoundEnded,
            Self::PhaseStarted(_) => EventKind::PhaseStarted,
            Self::TurnStarted(_) => EventKind::TurnStarted,
            Self::TurnEnded(_) => EventKind::TurnEnded,
            Self::Passed(_) => EventKind::Passed,
            Self::EntityMoved { .. } => EventKind::EntityMoved,
            Self::Shuffled(_) => EventKind::Shuffled,
            Self::CardBought { .. } => EventKind::CardBought,
            Self::AbilityActivated { .. } => EventKind::AbilityActivated,
            Self::ResourceGained { .. } => EventKind::ResourceGained,
            Self::ResourcePaid { .. } => EventKind::ResourcePaid,
            Self::CounterChanged { .. } => EventKind::CounterChanged,
            Self::DecisionRequested(_) => EventKind::DecisionRequested,
            Self::DecisionMade(_) => EventKind::DecisionMade,
            Self::GameEnded(_) => EventKind::GameEnded,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_ignores_the_payload() {
        assert_eq!(
            Event::TurnStarted(PlayerId(0)).kind(),
            Event::TurnStarted(PlayerId(3)).kind()
        );
        assert_ne!(
            Event::TurnStarted(PlayerId(0)).kind(),
            Event::TurnEnded(PlayerId(0)).kind()
        );
        let moved = Event::EntityMoved {
            entity: EntityId(0),
            from:   ZoneId(0),
            to:     ZoneId(1),
        };
        assert_eq!(moved.kind(), EventKind::EntityMoved);
    }
}
