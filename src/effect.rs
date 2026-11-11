//! Effects, abilities and the resolution machinery's data (tasks and pending
//! decisions).
//!
//! `Custom(fn)` escape hatches from the plan are left out on purpose: function
//! pointers cannot be serialized.

use serde::{Deserialize, Serialize};

use crate::{
    card::Discount,
    cost::{Cost, Count, Payment},
    event::EventKind,
    id::{CounterId, EntityId, PhaseId, PlayerId, ReactKind, ResourceId, ZoneKind},
    modifier::{Duration, RuleChange, Subject},
    state::GameState,
    zone::Owner,
};

/// Who or what an effect points at, relative to the ability that produced it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    /// The player controlling the ability.
    Controller,
    /// The card carrying the ability.
    Source,
    /// A specific player.
    Player(PlayerId),
    /// A specific card.
    Entity(EntityId),
    /// The card picked by an enclosing [`Effect::Pick`] /
    /// [`Effect::ForEachOwned`], or paid as part of the cost (destroyed /
    /// discarded).
    Card,
}

/// Which owner's zone or counter an effect refers to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OwnerRef {
    /// The controlling player.
    Controller,
    /// The shared owner.
    Shared,
    /// A specific player.
    Player(PlayerId),
    /// A specific team.
    Players(Vec<PlayerId>),
}

/// Values bound while an effect resolves: the card being worked on, the number
/// chosen while paying, and the single resource paid.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bound {
    /// The picked / destroyed / discarded card.
    pub card:      Option<EntityId>,
    /// The first card discarded as part of the cost (also `card` when nothing
    /// was destroyed).
    pub discarded: Option<EntityId>,
    /// The variable number chosen while paying.
    pub x:         u32,
    /// The resource paid when exactly one unit was paid.
    pub paid:      Option<ResourceId>,
}

/// Who is resolving an effect and from which card.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ctx {
    /// The card the effect comes from, if any.
    pub source:     Option<EntityId>,
    /// The player in control.
    pub controller: PlayerId,
    /// Values bound so far.
    pub bound:      Bound,
}

impl Ctx {
    /// A context with nothing bound.
    pub const fn new(source: Option<EntityId>, controller: PlayerId) -> Self {
        Self {
            source,
            controller,
            bound: Bound {
                card:      None,
                discarded: None,
                x:         0,
                paid:      None,
            },
        }
    }

    /// The same context working on another card.
    #[must_use]
    pub const fn with_card(mut self, card: EntityId) -> Self {
        self.bound.card = Some(card);
        self
    }

    /// The same context for another controller.
    #[must_use]
    pub const fn for_player(mut self, player: PlayerId) -> Self {
        self.controller = player;
        self
    }

    /// Resolves a target; `None` for `Source` / `Card` when there is none.
    pub const fn subject(&self, target: Target) -> Option<Subject> {
        match target {
            Target::Controller => Some(Subject::Player(self.controller)),
            Target::Source => match self.source {
                Some(e) => Some(Subject::Entity(e)),
                None => None,
            },
            Target::Card => match self.bound.card {
                Some(e) => Some(Subject::Entity(e)),
                None => None,
            },
            Target::Player(p) => Some(Subject::Player(p)),
            Target::Entity(e) => Some(Subject::Entity(e)),
        }
    }

    /// Resolves an owner reference.
    pub fn owner(&self, owner: &OwnerRef) -> Owner {
        match owner {
            OwnerRef::Controller => Owner::Player(self.controller),
            OwnerRef::Shared => Owner::Shared,
            OwnerRef::Player(p) => Owner::Player(*p),
            OwnerRef::Players(team) => Owner::Players(team.clone()),
        }
    }
}

/// Selects cards in a zone.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Filter {
    /// Must carry this tag.
    pub tag:           Option<String>,
    /// Must be exhausted (`true`) or ready (`false`).
    pub exhausted:     Option<bool>,
    /// Must hold (`true`) or not hold (`false`) resources on the card.
    pub has_resources: Option<bool>,
}

/// Which players an effect hits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Who {
    /// Everybody except the controller.
    Rivals,
    /// Everybody, controller included.
    Everyone,
}

/// A way for an affected player to avoid an effect, offered by the effect
/// itself rather than by a card.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Escape {
    /// Pay this resource.
    Pay(ResourceId, u32),
    /// Discard a card from hand.
    DiscardCard,
    /// Destroy one of your cards carrying this tag.
    Destroy(String),
}

/// How an affected player may avoid an effect aimed at them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Defense {
    /// Their React abilities of these kinds may be used instead.
    pub react:       Vec<ReactKind>,
    /// A further way offered by the effect itself.
    pub escape:      Option<Escape>,
    /// Players that already passed are not affected at all.
    pub skip_passed: bool,
}

/// Something that changes the game state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Effect {
    /// Take resources from the bank (capped to what it holds).
    Gain {
        /// Receiving player or card.
        to:  Target,
        /// Resource type.
        res: ResourceId,
        /// Amount.
        n:   u32,
    },
    /// Take as many of a resource as the variable number chosen while paying.
    GainX {
        /// Receiving player or card.
        to:  Target,
        /// Resource type.
        res: ResourceId,
    },
    /// The player picks a mix of resources (any kinds not excluded, or one
    /// kind when `identical`) and takes them.
    GainChoose {
        /// Receiving player or card.
        to:        Target,
        /// How many.
        n:         Count,
        /// Kinds that cannot be taken.
        exclude:   Vec<ResourceId>,
        /// All of the same kind.
        identical: bool,
    },
    /// Take the resource paid as part of the cost (one unit).
    GainPaid {
        /// Receiving player or card.
        to: Target,
    },
    /// Every affected player takes resources from the bank.
    GainAll {
        /// Who gains.
        who: Who,
        /// Resource type.
        res: ResourceId,
        /// Amount.
        n:   u32,
    },
    /// The controller picks a rival and gains as many `res` as that rival holds
    /// of `counted`.
    GainPerRival {
        /// Resource gained.
        res:     ResourceId,
        /// Resource counted in the rival's pool.
        counted: ResourceId,
    },
    /// Pay resources from the controller's pool to the bank.
    Pay {
        /// Resource type.
        res: ResourceId,
        /// Amount.
        n:   u32,
    },
    /// Draw cards between two zones of an owner (reshuffling when the source
    /// has a recycle zone).
    Draw {
        /// Whose zones.
        who:  OwnerRef,
        /// Source zone kind.
        from: ZoneKind,
        /// Destination zone kind.
        to:   ZoneKind,
        /// Number of cards.
        n:    u32,
    },
    /// Move one card to the top of a zone.
    Move {
        /// The card (a card target: `Source`, `Entity` or `Card`).
        entity: Target,
        /// Destination zone kind.
        to:     ZoneKind,
        /// Destination zone owner.
        owner:  OwnerRef,
    },
    /// Move every card of one zone to the top of another.
    MoveAll {
        /// Source zone kind.
        from:  ZoneKind,
        /// Destination zone kind.
        to:    ZoneKind,
        /// Owner of both zones.
        owner: OwnerRef,
    },
    /// Move the card at a position of one zone to the top of another zone of
    /// the same owner (what a player picked blindly, e.g. a dealt card).
    MoveNth {
        /// Source zone kind.
        from:  ZoneKind,
        /// Position in the source zone.
        index: usize,
        /// Destination zone kind.
        to:    ZoneKind,
        /// Owner of both zones.
        owner: OwnerRef,
    },
    /// Add to a counter (clamped to its bounds).
    ModifyCounter {
        /// The counter.
        counter: CounterId,
        /// Its owner.
        owner:   OwnerRef,
        /// Change.
        delta:   i64,
    },
    /// Set every counter with this id back to its minimum.
    ResetCounters {
        /// The counter.
        counter: CounterId,
    },
    /// Add a modifier.
    AddModifier {
        /// What it applies to.
        target:   Target,
        /// The rule change.
        rule:     RuleChange,
        /// How long it lasts.
        duration: Duration,
    },
    /// Turn a card sideways.
    Exhaust(Target),
    /// Straighten a card.
    Ready(Target),
    /// Straighten every turned card in the owner's active zones.
    ReadyAll {
        /// Whose cards.
        owner: OwnerRef,
    },
    /// Move every resource on a card into the controller's pool.
    TakeAll {
        /// The card.
        from: Target,
    },
    /// Add `per_kind` of every resource kind already on the source card.
    Grow {
        /// Amount added per kind present.
        per_kind: u32,
    },
    /// Destroy a card: its resources go back to the bank and it goes to the
    /// discard pile of its owner.
    Destroy {
        /// The card.
        target: Target,
    },
    /// The controller picks one of the effects, which then resolves.
    Choose(Vec<Effect>),
    /// Resolve the effects one after another.
    Sequence(Vec<Effect>),
    /// Resolve the effect once per player, in turn order, with that player as
    /// controller.
    ForEachPlayer(Box<Effect>),
    /// Resolve the effect once per card in the controller's active zones that
    /// passes the filter, with that card bound.
    ForEachOwned {
        /// Which cards.
        filter: Filter,
        /// What to do with each.
        effect: Box<Effect>,
    },
    /// The controller picks a card from a zone (passing the filter); the
    /// effect then resolves with that card bound. With one candidate there is
    /// no decision, with none nothing happens.
    Pick {
        /// Zone kind.
        from:   ZoneKind,
        /// Zone owner.
        owner:  OwnerRef,
        /// Which cards may be picked.
        filter: Filter,
        /// What to do with the picked card.
        then:   Box<Effect>,
    },
    /// Resolve the effect with a given card bound (the options built by
    /// [`Effect::Pick`]).
    WithCard {
        /// The card.
        card:   EntityId,
        /// What to do.
        effect: Box<Effect>,
    },
    /// The effect is aimed at every affected player, in turn order starting
    /// after the controller (with `Everyone` the controller comes last). Each
    /// of them resolves `effect` as its controller unless they avoid it through
    /// the [`Defense`]. The avoiding option does nothing by itself: a React
    /// ability's own effect is what happens instead.
    Affect {
        /// Who is aimed at.
        who:     Who,
        /// How they may avoid it.
        defense: Defense,
        /// What happens to them otherwise.
        effect:  Box<Effect>,
    },
    /// One affected player's share of an [`Effect::Affect`].
    AffectOne {
        /// The affected player.
        player:  PlayerId,
        /// How they may avoid it.
        defense: Defense,
        /// What happens to them otherwise.
        effect:  Box<Effect>,
    },
    /// The controller loses `n` of a resource. For each unit they cannot lose
    /// they lose `shortfall` other resources of their choice instead, as far as
    /// they have them.
    Lose {
        /// The resource.
        res:       ResourceId,
        /// Amount.
        n:         u32,
        /// Other resources lost per missing unit.
        shortfall: u32,
    },
    /// The controller uses an ability of `source`: pays `payment`, then the
    /// ability's effect resolves with that card as source and the payment's
    /// values bound. It does not count as an action (the options built by
    /// reaction windows).
    Use {
        /// The card carrying the ability.
        source:  EntityId,
        /// Index of the ability on the card.
        ability: usize,
        /// What is paid.
        payment: Payment,
    },
    /// Does nothing; the effect of a React ability that only avoids something.
    Ignore,
    /// Offers the controller their React abilities of this kind; if they use
    /// one the `default` does not happen. With `repeat` the offer is made
    /// again after each reaction, and declining is `default` doing nothing.
    ReactWindow {
        /// Which reactions.
        kind:    ReactKind,
        /// What happens when no reaction is used.
        default: Box<Effect>,
        /// Offer again after a reaction.
        repeat:  bool,
    },
    /// Evaluates the game's win condition now.
    CheckWin,
    /// The controller picks how to reorder the top `n` cards of a zone (put
    /// back as they were, or in another order).
    Reorder {
        /// Zone owner.
        owner: OwnerRef,
        /// Zone kind.
        zone:  ZoneKind,
        /// How many cards.
        n:     usize,
    },
    /// Reorders the top cards of a zone by a permutation (the options built by
    /// [`Effect::Reorder`]).
    Permute {
        /// Zone owner.
        owner: OwnerRef,
        /// Zone kind.
        zone:  ZoneKind,
        /// New order of the top cards, as indices into the old order.
        perm:  Vec<usize>,
    },
    /// The controller places a card from their hand (or another of their
    /// zones) into play, paying its placement cost minus the discounts they
    /// have plus this extra one (a huge `n` makes it free).
    PlaceCard {
        /// Where the card comes from; `None` is the hand.
        from:     Option<ZoneKind>,
        /// Only cards with this tag.
        tag:      Option<String>,
        /// The extra discount, `None` for the full price.
        discount: Option<Discount>,
    },
    /// The controller discards `n` cards of their choice from hand.
    DiscardChoose {
        /// How many.
        n: u32,
    },
    /// The controller draws up to `n` cards, then discards as many cards of
    /// their choice from hand as were really drawn (fewer when the deck and
    /// its recycle pile run short).
    DrawDiscard {
        /// Source zone kind.
        from: ZoneKind,
        /// Destination zone kind (the hand the cards are discarded from).
        to:   ZoneKind,
        /// How many to draw.
        n:    u32,
    },
}

/// What makes an ability happen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Trigger {
    /// The player activates it as an action (pays the cost).
    Activated,
    /// It happens by itself when an event of this kind occurs (no cost).
    OnEvent(EventKind),
    /// The owner may use it out of turn in reaction to something (pays the
    /// cost).
    React(ReactKind),
}

/// Extra requirement on top of the trigger, from the point of view of the
/// ability's controller.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Condition {
    /// No requirement.
    Always,
    /// Only during the controller's own turn.
    OwnTurn,
    /// Only during somebody else's turn.
    OtherTurn,
    /// Only while this phase is running (e.g. a "collect" ability on the start
    /// of the collect phase).
    InPhase(PhaseId),
    /// Only while the source card holds at least one of this resource.
    SourceHas(ResourceId),
    /// Only while the source card holds any resources.
    SourceHasResources,
    /// Every condition holds.
    All(Vec<Condition>),
}

impl Condition {
    /// Whether the condition holds for `controller` and the card `source`.
    pub fn holds(&self, state: &GameState, controller: PlayerId, source: Option<EntityId>) -> bool {
        let active = state.flow.active_player();
        let held = |f: &dyn Fn(&crate::resource::Pool) -> bool| {
            source.and_then(|e| state.entity(e).ok()).is_some_and(|e| f(&e.pool))
        };
        match self {
            Self::Always => true,
            Self::OwnTurn => active == Some(controller),
            Self::OtherTurn => active.is_some_and(|a| a != controller),
            Self::InPhase(wanted) => state.flow.phase_id() == Some(*wanted),
            Self::SourceHas(res) => held(&|pool| pool.finite(*res).is_none_or(|n| n > 0)),
            Self::SourceHasResources => {
                held(&|pool| pool.iter().any(|(_, s)| s != crate::resource::Supply::Finite(0)))
            },
            Self::All(all) => all.iter().all(|c| c.holds(state, controller, source)),
        }
    }
}

/// How often an ability may be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Limit {
    /// Unlimited.
    None,
    /// Once per turn (whoever's turn it is).
    OncePerTurn,
    /// Once per round.
    OncePerRound,
    /// Once between the controller's turns (usable again when their turn
    /// starts).
    OncePerBetweenTurns,
}

/// A trigger, a condition, an optional cost, an effect and a usage limit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ability {
    /// What makes it happen.
    pub trigger:   Trigger,
    /// Extra requirement.
    pub condition: Condition,
    /// Cost of an [`Trigger::Activated`] or [`Trigger::React`] ability (ignored
    /// for triggered ones).
    pub cost:      Option<Cost>,
    /// What it does.
    pub effect:    Effect,
    /// Usage limit.
    pub limit:     Limit,
}

/// Record that a limited ability was used, until the limit resets.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AbilityUse {
    /// The card.
    pub entity:     EntityId,
    /// Index of the ability on the card.
    pub ability:    usize,
    /// The player who controlled it when used.
    pub controller: PlayerId,
    /// The limit being enforced.
    pub limit:      Limit,
}

impl AbilityUse {
    /// Whether the event resets this record.
    pub fn resets_on(&self, event: &crate::event::Event) -> bool {
        use crate::event::Event;
        match self.limit {
            Limit::None => true,
            Limit::OncePerTurn => matches!(event, Event::TurnEnded(_)),
            Limit::OncePerRound => matches!(event, Event::RoundEnded(_)),
            Limit::OncePerBetweenTurns => {
                matches!(event, Event::TurnStarted(p) if *p == self.controller)
            },
        }
    }
}

/// The engine waiting for decisions. Several players can decide at the same
/// time (drafts, secret bids); the chosen options only resolve once everyone
/// answered.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingDecision {
    /// Players that must decide.
    pub players: Vec<PlayerId>,
    /// The options, each an effect that resolves for the player who picked it.
    pub options: Vec<Effect>,
    /// Answers so far as `(player, option index)`.
    pub answers: Vec<(PlayerId, usize)>,
    /// The card the decision comes from.
    pub source:  Option<EntityId>,
    /// Values bound when the decision was requested.
    pub bound:   Bound,
}

impl PendingDecision {
    /// Whether the player still has to answer.
    pub fn waiting_for(&self, player: PlayerId) -> bool {
        self.players.contains(&player) && !self.answers.iter().any(|(p, _)| *p == player)
    }
}

/// A unit of work on the resolution stack.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Task {
    /// Resolve an effect.
    Effect {
        /// The effect.
        effect: Effect,
        /// Who resolves it.
        ctx:    Ctx,
    },
    /// End the current player's turn.
    EndTurn,
    /// Hand the turn to the next player, or end the phase.
    NextTurn,
    /// Enter the next phase (or the next round).
    NextPhase,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{event::Event, rng::Rng};

    const P0: PlayerId = PlayerId(0);
    const P1: PlayerId = PlayerId(1);

    #[test]
    fn ctx_resolves_relative_targets() {
        let ctx = Ctx::new(Some(EntityId(4)), P1);
        assert_eq!(ctx.subject(Target::Controller), Some(Subject::Player(P1)));
        assert_eq!(
            ctx.subject(Target::Source),
            Some(Subject::Entity(EntityId(4)))
        );
        assert_eq!(ctx.subject(Target::Card), None);
        assert_eq!(
            ctx.with_card(EntityId(9)).subject(Target::Card),
            Some(Subject::Entity(EntityId(9)))
        );
        assert_eq!(ctx.owner(&OwnerRef::Controller), Owner::Player(P1));
        assert_eq!(ctx.owner(&OwnerRef::Shared), Owner::Shared);
        let team = vec![P0, P1];
        assert_eq!(
            ctx.owner(&OwnerRef::Players(team.clone())),
            Owner::Players(team)
        );
        assert_eq!(Ctx::new(None, P0).subject(Target::Source), None);
        assert_eq!(ctx.for_player(P0).controller, P0);
    }

    #[test]
    fn conditions_depend_on_whose_turn_it_is_and_the_phase() {
        use crate::flow::{Flow, Phase, TurnOrder, TurnOrderPolicy};
        let mut s = GameState::new(Rng::from_seed(1));
        s.flow = Flow {
            phases: vec![Phase {
                id:          PhaseId(2),
                interactive: true,
            }],
            phase: Some(0),
            turn_order: TurnOrder::new(vec![P0, P1], TurnOrderPolicy::Clockwise),
            ..Flow::default()
        };
        assert!(Condition::OwnTurn.holds(&s, P0, None));
        assert!(!Condition::OwnTurn.holds(&s, P1, None));
        assert!(Condition::OtherTurn.holds(&s, P1, None));
        assert!(Condition::Always.holds(&s, P0, None));
        assert!(Condition::InPhase(PhaseId(2)).holds(&s, P0, None));
        assert!(!Condition::InPhase(PhaseId(1)).holds(&s, P0, None));
        let both = Condition::All(vec![Condition::OwnTurn, Condition::InPhase(PhaseId(1))]);
        assert!(!both.holds(&s, P0, None));
    }

    #[test]
    fn source_conditions_look_at_the_resources_on_the_card() {
        use crate::{
            card::CardDef,
            id::DefId,
            zone::{Order, Visibility},
        };
        let mut s = GameState::new(Rng::from_seed(1));
        s.add_def(CardDef {
            id:        DefId(0),
            name:      "c".into(),
            tags:      vec![],
            cost:      None,
            on_gain:   None,
            abilities: vec![],
            discounts: vec![],
        });
        s.bank.set_infinite(ResourceId(0));
        let z = s.add_zone(
            ZoneKind(0),
            Owner::Shared,
            Order::Unordered,
            Visibility::Public,
        );
        let e = s.create_entity(DefId(0), z, true).unwrap();
        assert!(!Condition::SourceHasResources.holds(&s, P0, Some(e)));
        assert!(!Condition::SourceHas(ResourceId(0)).holds(&s, P0, Some(e)));
        s.gain_entity(e, ResourceId(0), 2).unwrap();
        assert!(Condition::SourceHasResources.holds(&s, P0, Some(e)));
        assert!(Condition::SourceHas(ResourceId(0)).holds(&s, P0, Some(e)));
        assert!(!Condition::SourceHas(ResourceId(1)).holds(&s, P0, Some(e)));
        assert!(!Condition::SourceHasResources.holds(&s, P0, None));
    }

    #[test]
    fn limits_reset_on_their_own_events() {
        let used = |limit| AbilityUse {
            entity: EntityId(0),
            ability: 0,
            controller: P0,
            limit,
        };
        assert!(used(Limit::OncePerTurn).resets_on(&Event::TurnEnded(P1)));
        assert!(!used(Limit::OncePerTurn).resets_on(&Event::RoundEnded(1)));
        assert!(used(Limit::OncePerRound).resets_on(&Event::RoundEnded(1)));
        assert!(used(Limit::OncePerBetweenTurns).resets_on(&Event::TurnStarted(P0)));
        assert!(!used(Limit::OncePerBetweenTurns).resets_on(&Event::TurnStarted(P1)));
    }

    #[test]
    fn pending_decision_tracks_who_is_still_waiting() {
        let mut d = PendingDecision {
            players: vec![P0, P1],
            options: vec![],
            answers: vec![],
            source:  None,
            bound:   Bound::default(),
        };
        assert!(d.waiting_for(P0) && d.waiting_for(P1));
        d.answers.push((P0, 0));
        assert!(!d.waiting_for(P0) && d.waiting_for(P1));
        assert!(
            !PendingDecision {
                players: vec![P0],
                ..d
            }
            .waiting_for(P1)
        );
    }
}
