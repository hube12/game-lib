//! Modifiers: temporary or permanent rule changes ("curses", conditions).

use serde::{Deserialize, Serialize};

use crate::{
    event::{Event, EventKind},
    id::{EntityId, PlayerId, ResourceId},
};

/// What a modifier applies to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Subject {
    /// A player.
    Player(PlayerId),
    /// A card.
    Entity(EntityId),
}

/// The kind of action a modifier can forbid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionKind {
    /// Buying from a market.
    Buy,
    /// Activating abilities.
    Activate,
}

/// The change a modifier makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuleChange {
    /// The subject cannot take this kind of action (a player, or a card for
    /// activations).
    Forbid(ActionKind),
    /// Costs paid by the player, or for the card, change by `delta` of one
    /// resource (never below zero).
    CostDelta {
        /// The resource.
        res:   ResourceId,
        /// Amount added to the cost (negative = discount).
        delta: i32,
    },
}

/// How long a modifier lasts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Duration {
    /// Until the end of the current turn.
    ThisTurn,
    /// During that player's next turn only (inactive until it starts).
    NextTurnOf(PlayerId),
    /// Until the end of the round.
    UntilRoundEnd,
    /// Until the next event of this kind.
    UntilEvent(EventKind),
    /// Forever.
    Permanent,
}

/// A live rule change.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Modifier {
    /// Who or what it applies to.
    pub target:   Subject,
    /// The change.
    pub rule:     RuleChange,
    /// How long it lasts.
    pub duration: Duration,
    /// For [`Duration::NextTurnOf`]: whether that turn has started.
    pub started:  bool,
}

impl Modifier {
    /// A fresh modifier.
    pub const fn new(target: Subject, rule: RuleChange, duration: Duration) -> Self {
        Self {
            target,
            rule,
            duration,
            started: false,
        }
    }

    /// Whether the rule change currently applies.
    pub const fn is_active(&self) -> bool {
        match self.duration {
            Duration::NextTurnOf(_) => self.started,
            _ => true,
        }
    }

    /// Feeds an event; returns whether the modifier survives it.
    pub fn update(&mut self, event: &Event) -> bool {
        match self.duration {
            Duration::ThisTurn => !matches!(event, Event::TurnEnded(_)),
            Duration::NextTurnOf(player) => {
                if matches!(event, Event::TurnStarted(p) if *p == player) {
                    self.started = true;
                }
                !(self.started && matches!(event, Event::TurnEnded(p) if *p == player))
            },
            Duration::UntilRoundEnd => !matches!(event, Event::RoundEnded(_)),
            Duration::UntilEvent(kind) => event.kind() != kind,
            Duration::Permanent => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const P0: PlayerId = PlayerId(0);
    const P1: PlayerId = PlayerId(1);

    fn modifier(duration: Duration) -> Modifier {
        Modifier::new(
            Subject::Player(P1),
            RuleChange::Forbid(ActionKind::Buy),
            duration,
        )
    }

    #[test]
    fn this_turn_ends_with_the_turn() {
        let mut m = modifier(Duration::ThisTurn);
        assert!(m.is_active());
        assert!(m.update(&Event::Passed(P0)));
        assert!(!m.update(&Event::TurnEnded(P0)));
    }

    #[test]
    fn next_turn_of_waits_then_lasts_one_turn() {
        let mut m = modifier(Duration::NextTurnOf(P1));
        assert!(!m.is_active());
        assert!(m.update(&Event::TurnEnded(P0)), "other turns do not end it");
        assert!(m.update(&Event::TurnStarted(P0)));
        assert!(!m.is_active());
        assert!(m.update(&Event::TurnStarted(P1)));
        assert!(m.is_active());
        assert!(!m.update(&Event::TurnEnded(P1)));
    }

    #[test]
    fn until_round_end_and_until_event() {
        let mut m = modifier(Duration::UntilRoundEnd);
        assert!(m.update(&Event::TurnEnded(P0)));
        assert!(!m.update(&Event::RoundEnded(1)));
        let mut m = modifier(Duration::UntilEvent(EventKind::Passed));
        assert!(m.update(&Event::TurnEnded(P0)));
        assert!(!m.update(&Event::Passed(P1)));
    }

    #[test]
    fn permanent_never_expires() {
        let mut m = modifier(Duration::Permanent);
        assert!(m.update(&Event::RoundEnded(1)));
        assert!(m.update(&Event::TurnEnded(P1)));
    }
}
