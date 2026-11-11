//! The engine: validates actions, applies them, and resolves effects, triggers
//! and decisions.
//!
//! Everything that happens goes through one loop. Actions push [`Task`]s on the
//! state's resolution stack; every state change queues an [`Event`]; after each
//! step the queued events are processed (modifiers expire, abilities trigger,
//! the win condition is checked) and triggered effects go on top of the stack.
//! The loop stops when the stack is empty or when a [`PendingDecision`] needs
//! input.

use serde::{Deserialize, Serialize};

use crate::{
    card::Discount,
    cost::{Cost, Count, Payment},
    effect::{
        Ability,
        AbilityUse,
        Bound,
        Ctx,
        Defense,
        Effect,
        Escape,
        Filter,
        Limit,
        OwnerRef,
        PendingDecision,
        Target,
        Task,
        Trigger,
        Who,
    },
    event::Event,
    id::{CounterId, EntityId, PhaseId, PlayerId, ReactKind, ResourceId, ZoneKind},
    market::MarketError,
    modifier::{ActionKind, Modifier, RuleChange, Subject},
    pay,
    resource::Supply,
    rng::Rng,
    state::{GameState, StateError},
    view::PlayerView,
    zone::{Owner, Pos},
};

/// Upper bound on resolution steps per action; a rules loop (an ability
/// triggering itself forever) hits it.
const MAX_STEPS: usize = 100_000;

/// Something a player can do.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    /// Pass for the rest of the phase; also ends the turn.
    Pass,
    /// End the turn without passing.
    EndTurn,
    /// Buy a card from a market.
    Buy {
        /// The card.
        entity: EntityId,
        /// Which way of paying, as an index into the cost's payments.
        pay:    usize,
    },
    /// Use an activated ability.
    Activate {
        /// The card carrying the ability.
        entity:  EntityId,
        /// Index of the ability on the card.
        ability: usize,
        /// Which way of paying, as an index into the cost's payments.
        pay:     usize,
    },
    /// Place a card from hand into play, paying its cost minus the owner's
    /// discounts. Needs `GameState::card_rules`.
    Place {
        /// The card in hand.
        entity: EntityId,
        /// Which way of paying, as an index into `pay::place_payments`.
        pay:    usize,
    },
    /// Discard a card from hand, then resolve `CardRules::on_discard`. Needs
    /// `GameState::card_rules`.
    Discard {
        /// The card in hand.
        entity: EntityId,
    },
    /// Answer the pending decision by picking an option.
    Decide(usize),
}

/// Why an action or a resolution step failed. On error the engine state is left
/// untouched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuleError {
    /// The game is over.
    GameOver,
    /// The action is not in the list of legal actions.
    Illegal,
    /// A state operation failed (a rules bug: bad zone kind, missing def, ...).
    State(StateError),
    /// A market operation failed.
    Market(MarketError),
    /// An effect used `Source` but there is no source card, or a card target
    /// was a player.
    BadTarget,
    /// An effect named a counter that does not exist.
    NoCounter(CounterId, Owner),
    /// The game has no phases.
    NoPhases,
    /// Resolution did not terminate within the step budget.
    Runaway,
    /// The effect is part of the engine extension that is not implemented yet
    /// (see `plan.md`).
    Unsupported,
}

impl From<StateError> for RuleError {
    fn from(e: StateError) -> Self {
        Self::State(e)
    }
}

impl From<MarketError> for RuleError {
    fn from(e: MarketError) -> Self {
        Self::Market(e)
    }
}

/// The game-specific part: how to set up a table, and the few places where a
/// game customizes the engine.
pub trait GameRules {
    /// Builds the initial state for these players. The generator must be moved
    /// into the state, so that shuffles during setup and play share one
    /// deterministic stream.
    fn setup(&self, players: &[PlayerId], rng: Rng) -> GameState;

    /// Actions available to a player. Defaults to [`standard_legal_actions`].
    fn legal_actions(&self, state: &GameState, player: PlayerId) -> Vec<Action> {
        standard_legal_actions(state, player)
    }

    /// The winners now, if the game is decided. Used by [`Effect::CheckWin`];
    /// the default applies the flow's
    /// [`GameEndPolicy`](crate::flow::GameEndPolicy) whatever its `when`.
    fn winners(&self, state: &GameState) -> Option<Vec<PlayerId>> {
        let policy = state.flow.game_end.as_ref()?;
        policy.winners(&state.counters, state.flow.turn_order.order())
    }

    /// Effects resolved when a phase starts (system steps that have no card
    /// abilities). Abilities triggered by the phase's start event resolve
    /// first. The first player in turn order is their controller.
    fn phase_effects(&self, _state: &GameState, _phase: PhaseId) -> Vec<Effect> {
        Vec::new()
    }

    /// Effects resolved when a player passes, before their turn ends. The pass
    /// is already recorded (`flow.turn_order.passed()`).
    fn pass_effects(&self, _state: &GameState, _player: PlayerId) -> Vec<Effect> {
        Vec::new()
    }

    /// Effects resolved at the end of a player's turn (discard hand, draw new
    /// cards, ...).
    fn turn_end_effects(&self, _state: &GameState, _player: PlayerId) -> Vec<Effect> {
        Vec::new()
    }
}

/// A reference to rules is rules too, so a tool can run a throwaway engine
/// without copying the game's data.
impl<R: GameRules + ?Sized> GameRules for &R {
    fn setup(&self, players: &[PlayerId], rng: Rng) -> GameState {
        (**self).setup(players, rng)
    }

    fn legal_actions(&self, state: &GameState, player: PlayerId) -> Vec<Action> {
        (**self).legal_actions(state, player)
    }

    fn winners(&self, state: &GameState) -> Option<Vec<PlayerId>> {
        (**self).winners(state)
    }

    fn phase_effects(&self, state: &GameState, phase: PhaseId) -> Vec<Effect> {
        (**self).phase_effects(state, phase)
    }

    fn pass_effects(&self, state: &GameState, player: PlayerId) -> Vec<Effect> {
        (**self).pass_effects(state, player)
    }

    fn turn_end_effects(&self, state: &GameState, player: PlayerId) -> Vec<Effect> {
        (**self).turn_end_effects(state, player)
    }
}

fn modifiers_on(
    state: &GameState,
    player: PlayerId,
    entity: Option<EntityId>,
) -> impl Iterator<Item = &Modifier> {
    state.modifiers.iter().filter(move |m| {
        m.is_active()
            && (m.target == Subject::Player(player)
                || entity.is_some_and(|e| m.target == Subject::Entity(e)))
    })
}

fn forbidden(state: &GameState, kind: ActionKind, player: PlayerId, entity: EntityId) -> bool {
    modifiers_on(state, player, Some(entity)).any(|m| m.rule == RuleChange::Forbid(kind))
}

/// The ways of paying a cost right now, after cost modifiers. `None` means free
/// (one empty payment).
pub fn payments(
    state: &GameState,
    player: PlayerId,
    source: Option<EntityId>,
    cost: Option<&Cost>,
    entity: EntityId,
) -> Vec<Payment> {
    let mut all = pay::enumerate(state, player, source, cost);
    for m in modifiers_on(state, player, Some(entity)) {
        if let RuleChange::CostDelta { res, delta } = m.rule {
            for payment in &mut all {
                payment.adjust(res, delta);
            }
        }
    }
    all
}

fn affordable(
    state: &GameState,
    player: PlayerId,
    source: Option<EntityId>,
    payment: &Payment,
) -> bool {
    let Ok(pool) = state.player_pool(player) else {
        return false;
    };
    let resources_ok = payment.resources.iter().all(|(res, n)| pool.has(*res, *n));
    let exhaust_ok =
        !payment.exhaust || source.and_then(|e| state.entity(e).ok()).is_some_and(|e| !e.exhausted);
    resources_ok && exhaust_ok
}

fn limit_used(state: &GameState, entity: EntityId, ability: usize, limit: Limit) -> bool {
    limit != Limit::None && state.used.iter().any(|u| u.entity == entity && u.ability == ability)
}

fn abilities_of(state: &GameState, entity: EntityId) -> &[Ability] {
    state
        .entity(entity)
        .ok()
        .and_then(|e| state.defs.get(&e.def))
        .map_or(&[], |d| d.abilities.as_slice())
}

/// The legal actions implied by the core rules: decide when a decision is
/// pending; otherwise, on your turn, end the turn, pass, buy affordable market
/// cards and activate affordable abilities.
pub fn standard_legal_actions(state: &GameState, player: PlayerId) -> Vec<Action> {
    if state.winners.is_some() {
        return Vec::new();
    }
    if let Some(pending) = &state.pending {
        return if pending.waiting_for(player) {
            (0..pending.options.len()).map(Action::Decide).collect()
        } else {
            Vec::new()
        };
    }
    let flow = &state.flow;
    if flow.active_player() != Some(player) {
        return Vec::new();
    }
    let mut actions = vec![Action::EndTurn, Action::Pass];

    for market in &state.markets {
        for entity in market.offered(state) {
            if forbidden(state, ActionKind::Buy, player, entity) {
                continue;
            }
            let cost = state
                .entity(entity)
                .ok()
                .and_then(|e| state.defs.get(&e.def))
                .and_then(|d| d.cost.as_ref());
            for (pay, payment) in payments(state, player, None, cost, entity).iter().enumerate() {
                if !payment.exhaust && affordable(state, player, None, payment) {
                    actions.push(Action::Buy { entity, pay });
                }
            }
        }
    }

    for zone in state.zones().filter(|z| z.abilities_active && z.owner.contains(player)) {
        for &entity in zone.entities() {
            if forbidden(state, ActionKind::Activate, player, entity) {
                continue;
            }
            for (ability, a) in abilities_of(state, entity).iter().enumerate() {
                if a.trigger != Trigger::Activated
                    || !a.condition.holds(state, player, Some(entity))
                    || limit_used(state, entity, ability, a.limit)
                {
                    continue;
                }
                for (pay, payment) in payments(state, player, Some(entity), a.cost.as_ref(), entity)
                    .iter()
                    .enumerate()
                {
                    if affordable(state, player, Some(entity), payment) {
                        actions.push(Action::Activate {
                            entity,
                            ability,
                            pay,
                        });
                    }
                }
            }
        }
    }
    if let Some(rules) = &state.card_rules
        && let Ok(hand) = state.zone_id(&Owner::Player(player), rules.hand)
        && let Ok(hand) = state.zone(hand)
    {
        for &entity in hand.entities() {
            for pay in 0..pay::place_payments(state, player, entity, None).len() {
                actions.push(Action::Place { entity, pay });
            }
            actions.push(Action::Discard { entity });
        }
    }
    actions
}

/// A running game: rules, state, event history and the action log that
/// reproduces it.
#[derive(Debug)]
pub struct Engine<R: GameRules> {
    rules:   R,
    state:   GameState,
    events:  Vec<Event>,
    log:     Vec<(PlayerId, Action)>,
    players: Vec<PlayerId>,
    seed:    u64,
}

impl<R: GameRules> Engine<R> {
    /// Sets up a game for `player_count` players (ids `0..player_count`) and
    /// runs it to the first decision.
    pub fn new(rules: R, player_count: u8, seed: u64) -> Result<Self, RuleError> {
        let players: Vec<_> = (0..player_count).map(PlayerId).collect();
        let mut state = rules.setup(&players, Rng::from_seed(seed));
        // Preparing the table must not trigger anything.
        state.clear_events();
        // At the bottom: tasks pushed by `setup` (drafting, picking) run first.
        state.stack.insert(0, Task::NextPhase);
        let mut engine = Self {
            rules,
            state,
            events: Vec::new(),
            log: Vec::new(),
            players,
            seed,
        };
        engine.settle()?;
        Ok(engine)
    }

    /// Continues from a state taken from another engine (a what-if copy for
    /// bots and tools). The copy starts with an empty event history and action
    /// log, so it cannot be replayed from a seed.
    pub fn from_state(rules: R, state: GameState) -> Self {
        let mut players = state.flow.turn_order.order().to_vec();
        players.sort();
        Self {
            rules,
            state,
            events: Vec::new(),
            log: Vec::new(),
            players,
            seed: 0,
        }
    }

    /// Plays a recorded game again from the same seed. Determinism makes the
    /// result identical to the original.
    pub fn replay(
        rules: R,
        player_count: u8,
        seed: u64,
        log: &[(PlayerId, Action)],
    ) -> Result<Self, RuleError> {
        let mut engine = Self::new(rules, player_count, seed)?;
        for (player, action) in log {
            engine.apply(*player, action.clone())?;
        }
        Ok(engine)
    }

    /// The current state.
    pub const fn state(&self) -> &GameState {
        &self.state
    }

    /// The rules.
    pub const fn rules(&self) -> &R {
        &self.rules
    }

    /// Every event since the game started.
    pub fn events(&self) -> &[Event] {
        &self.events
    }

    /// Every accepted action, in order. With the seed this reproduces the game.
    pub fn log(&self) -> &[(PlayerId, Action)] {
        &self.log
    }

    /// The seed the game was created with.
    pub const fn seed(&self) -> u64 {
        self.seed
    }

    /// The players of this game.
    pub fn players(&self) -> &[PlayerId] {
        &self.players
    }

    /// The player whose turn it is.
    pub fn current_player(&self) -> Option<PlayerId> {
        self.state.flow.active_player()
    }

    /// The decision the game waits for, if any.
    pub const fn pending(&self) -> Option<&PendingDecision> {
        self.state.pending.as_ref()
    }

    /// The winners once the game is over.
    pub fn winners(&self) -> Option<&[PlayerId]> {
        self.state.winners.as_deref()
    }

    /// What `player` may do now.
    pub fn legal_actions(&self, player: PlayerId) -> Vec<Action> {
        self.rules.legal_actions(&self.state, player)
    }

    /// What `player` is allowed to know.
    pub fn view(&self, player: PlayerId) -> PlayerView {
        PlayerView::new(&self.state, player)
    }

    /// Applies an action and returns the events it caused. On error nothing
    /// changes.
    pub fn apply(&mut self, player: PlayerId, action: Action) -> Result<Vec<Event>, RuleError> {
        if self.state.winners.is_some() {
            return Err(RuleError::GameOver);
        }
        if !self.legal_actions(player).contains(&action) {
            return Err(RuleError::Illegal);
        }
        let backup = self.state.clone();
        let first_event = self.events.len();
        if let Err(e) = self.execute(player, &action).and_then(|()| self.settle()) {
            self.state = backup;
            self.events.truncate(first_event);
            return Err(e);
        }
        self.log.push((player, action));
        Ok(self.events[first_event..].to_vec())
    }

    fn execute(&mut self, player: PlayerId, action: &Action) -> Result<(), RuleError> {
        match *action {
            Action::Pass => {
                self.state.flow.turn_order.pass(player);
                self.state.emit(Event::Passed(player));
                self.state.stack.push(Task::EndTurn);
                let effects = self.rules.pass_effects(&self.state, player);
                if !effects.is_empty() {
                    self.push_effect(Effect::Sequence(effects), Ctx::new(None, player));
                }
            },
            Action::EndTurn => self.state.stack.push(Task::EndTurn),
            Action::Buy { entity, pay } => self.buy(player, entity, pay)?,
            Action::Activate {
                entity,
                ability,
                pay,
            } => self.activate(player, entity, ability, pay)?,
            Action::Place { entity, pay } => self.place(player, entity, pay)?,
            Action::Discard { entity } => self.discard_action(player, entity)?,
            Action::Decide(choice) => self.decide(player, choice)?,
        }
        Ok(())
    }

    /// Counts an action; when the game limits actions per turn, the turn ends
    /// after the effects resolve.
    fn count_action(&mut self) {
        let flow = &mut self.state.flow;
        flow.actions_taken += 1;
        if flow.actions_per_turn.is_some_and(|n| flow.actions_taken >= n) {
            self.state.stack.push(Task::EndTurn);
        }
    }

    fn pay(
        &mut self,
        player: PlayerId,
        source: Option<EntityId>,
        payment: &Payment,
    ) -> Result<(), RuleError> {
        for (res, n) in &payment.resources {
            self.state.pay_player(player, *res, *n)?;
        }
        if payment.exhaust {
            let entity = source.ok_or(RuleError::BadTarget)?;
            self.state.entity_mut(entity)?.exhausted = true;
        }
        if !payment.from_card.is_empty() {
            let entity = source.ok_or(RuleError::BadTarget)?;
            for (res, n) in &payment.from_card {
                self.state.take_from_card(entity, *res, *n)?;
            }
        }
        for entity in &payment.turned {
            self.state.entity_mut(*entity)?.exhausted = true;
        }
        for entity in &payment.discarded {
            self.discard_card(*entity)?;
        }
        if let Some(entity) = payment.destroyed {
            self.destroy_card(entity)?;
        }
        Ok(())
    }

    /// Moves a card to the discard pile of the owner of the zone it is in.
    fn discard_card(&mut self, entity: EntityId) -> Result<(), RuleError> {
        let kind = self.state.card_rules.as_ref().ok_or(RuleError::Unsupported)?.discard;
        let owner = self.state.zone(self.state.entity(entity)?.zone)?.owner.clone();
        let to = self.state.zone_id(&owner, kind)?;
        self.state.move_entity(entity, to, Pos::Top)?;
        Ok(())
    }

    /// Destroys a card: its resources go back to the bank, then it is
    /// discarded.
    fn destroy_card(&mut self, entity: EntityId) -> Result<(), RuleError> {
        self.state.return_to_bank(entity)?;
        self.discard_card(entity)
    }

    /// The cards of a zone that pass the filter.
    fn candidates(
        &self,
        ctx: &Ctx,
        from: ZoneKind,
        owner: &OwnerRef,
        filter: &Filter,
    ) -> Result<Vec<EntityId>, RuleError> {
        let zone = self.state.zone_id(&ctx.owner(owner), from)?;
        let mut out = Vec::new();
        for &entity in self.state.zone(zone)?.entities() {
            let card = self.state.entity(entity)?;
            let holds = card.pool.iter().any(|(_, s)| s != crate::resource::Supply::Finite(0));
            let ok = filter.tag.as_ref().is_none_or(|t| pay::has_tag(&self.state, entity, t))
                && filter.exhausted.is_none_or(|x| x == card.exhausted)
                && filter.has_resources.is_none_or(|h| h == holds);
            if ok {
                out.push(entity);
            }
        }
        Ok(out)
    }

    /// The player of an entity-valued target.
    const fn card_of(ctx: &Ctx, target: Target) -> Result<EntityId, RuleError> {
        match ctx.subject(target) {
            Some(Subject::Entity(e)) => Ok(e),
            _ => Err(RuleError::BadTarget),
        }
    }

    fn buy(&mut self, player: PlayerId, entity: EntityId, pay: usize) -> Result<(), RuleError> {
        let card = self.state.entity(entity)?;
        let def = self.state.defs.get(&card.def).ok_or(StateError::NoDef(card.def))?;
        let (cost, on_gain) = (def.cost.clone(), def.on_gain.clone());
        let payment = payments(&self.state, player, None, cost.as_ref(), entity)
            .into_iter()
            .nth(pay)
            .ok_or(RuleError::Illegal)?;
        let market = self
            .state
            .markets
            .iter()
            .find(|m| m.holds(&self.state, entity))
            .cloned()
            .ok_or(RuleError::Illegal)?;
        self.pay(player, None, &payment)?;
        self.count_action();
        market.buy(&mut self.state, player, entity)?;
        self.state.emit(Event::CardBought { player, entity });
        if let Some(effect) = on_gain {
            self.push_effect(effect, Ctx::new(Some(entity), player));
        }
        Ok(())
    }

    fn activate(
        &mut self,
        player: PlayerId,
        entity: EntityId,
        index: usize,
        pay: usize,
    ) -> Result<(), RuleError> {
        let ability =
            abilities_of(&self.state, entity).get(index).cloned().ok_or(RuleError::Illegal)?;
        let payment = payments(
            &self.state,
            player,
            Some(entity),
            ability.cost.as_ref(),
            entity,
        )
        .into_iter()
        .nth(pay)
        .ok_or(RuleError::Illegal)?;
        // The end of the turn goes on the stack first so the ability resolves
        // before it.
        self.count_action();
        self.use_ability(player, entity, index, &payment)
    }

    /// Pays for an ability and queues its effect with the payment's values
    /// bound. Shared by actions and reactions.
    fn use_ability(
        &mut self,
        player: PlayerId,
        entity: EntityId,
        index: usize,
        payment: &Payment,
    ) -> Result<(), RuleError> {
        let ability =
            abilities_of(&self.state, entity).get(index).cloned().ok_or(RuleError::Illegal)?;
        self.pay(player, Some(entity), payment)?;
        if ability.limit != Limit::None {
            self.state.used.push(AbilityUse {
                entity,
                ability: index,
                controller: player,
                limit: ability.limit,
            });
        }
        self.state.emit(Event::AbilityActivated {
            player,
            entity,
            ability: index,
        });
        let mut ctx = Ctx::new(Some(entity), player);
        ctx.bound = Bound {
            card:      payment.destroyed.or_else(|| payment.discarded.first().copied()),
            discarded: payment.discarded.first().copied(),
            x:         payment.x.unwrap_or(0),
            paid:      payment.single_paid(),
        };
        self.push_effect(ability.effect, ctx);
        Ok(())
    }

    /// Moves a card from hand to the player's tableau, paying for it.
    fn place(&mut self, player: PlayerId, entity: EntityId, pay: usize) -> Result<(), RuleError> {
        let payment = pay::place_payments(&self.state, player, entity, None)
            .into_iter()
            .nth(pay)
            .ok_or(RuleError::Illegal)?;
        self.pay(player, None, &payment)?;
        self.count_action();
        self.put_in_play(player, entity)
    }

    fn put_in_play(&mut self, player: PlayerId, entity: EntityId) -> Result<(), RuleError> {
        let kind = self.state.card_rules.as_ref().ok_or(RuleError::Unsupported)?.tableau;
        let to = self.state.zone_id(&Owner::Player(player), kind)?;
        self.state.move_entity(entity, to, Pos::Top)?;
        Ok(())
    }

    /// The discard action: the card leaves the hand, then the game's reward
    /// resolves.
    fn discard_action(&mut self, player: PlayerId, entity: EntityId) -> Result<(), RuleError> {
        self.discard_card(entity)?;
        self.count_action();
        let reward = self.state.card_rules.as_ref().and_then(|r| r.on_discard.clone());
        if let Some(effect) = reward {
            self.push_effect(effect, Ctx::new(None, player));
        }
        Ok(())
    }

    /// Offers the controller every card in hand (optionally with a tag) they
    /// can pay for with the extra discount, paired with each way of paying,
    /// plus the choice to place nothing.
    fn place_card(
        &mut self,
        ctx: &Ctx,
        from: Option<ZoneKind>,
        tag: Option<&String>,
        discount: Option<&Discount>,
    ) -> Result<(), RuleError> {
        let rules = self.state.card_rules.clone().ok_or(RuleError::Unsupported)?;
        let source =
            self.state.zone_id(&Owner::Player(ctx.controller), from.unwrap_or(rules.hand))?;
        let mut options = Vec::new();
        for &card in self.state.zone(source)?.entities() {
            if tag.is_some_and(|t| !pay::has_tag(&self.state, card, t)) {
                continue;
            }
            for payment in pay::place_payments(&self.state, ctx.controller, card, discount) {
                let mut steps: Vec<_> = payment
                    .resources
                    .iter()
                    .map(|(res, n)| Effect::Pay { res: *res, n: *n })
                    .collect();
                steps.push(Effect::WithCard {
                    card,
                    effect: Box::new(Effect::Move {
                        entity: Target::Card,
                        to:     rules.tableau,
                        owner:  OwnerRef::Controller,
                    }),
                });
                options.push(Effect::Sequence(steps));
            }
        }
        if !options.is_empty() {
            options.push(Effect::Ignore);
            self.push_effect(Effect::Choose(options), *ctx);
        }
        Ok(())
    }

    /// How many resources the placement cost of a card consists of (0 for a
    /// free card or a cost that is not made of plain resources).
    fn placement_cost_units(&self, card: EntityId) -> u32 {
        fn units(cost: &Cost) -> u32 {
            match cost {
                Cost::Resources(parts) => parts.iter().map(|(_, n)| n).sum(),
                Cost::All(parts) => parts.iter().map(units).sum(),
                Cost::Any {
                    n: Count::Fixed(n), ..
                } => *n,
                _ => 0,
            }
        }
        self.state
            .entity(card)
            .ok()
            .and_then(|e| self.state.defs.get(&e.def))
            .and_then(|d| d.cost.as_ref())
            .map_or(0, units)
    }

    /// Whether an option can possibly resolve for the player now. Only what is
    /// cheap to know: paying more than the pool holds is impossible.
    fn feasible(&self, player: PlayerId, effect: &Effect) -> bool {
        let Ok(pool) = self.state.player_pool(player) else {
            return true;
        };
        match effect {
            Effect::Pay { res, n } => pool.has(*res, *n),
            Effect::Sequence(steps) => {
                let mut owed: std::collections::BTreeMap<ResourceId, u32> =
                    std::collections::BTreeMap::new();
                for step in steps {
                    if let Effect::Pay { res, n } = step {
                        *owed.entry(*res).or_insert(0) += n;
                    }
                }
                owed.iter().all(|(res, n)| pool.has(*res, *n))
            },
            _ => true,
        }
    }

    /// Asks the controller to pick among `options`; no option means nothing
    /// happens and a single one is applied without asking.
    fn choose_among(&mut self, options: Vec<Effect>, ctx: Ctx) {
        let mut options = options.into_iter().fold(Vec::new(), |mut unique, o| {
            if !unique.contains(&o) {
                unique.push(o);
            }
            unique
        });
        match options.len() {
            0 => {},
            1 => self.push_effect(options.remove(0), ctx),
            _ => self.push_effect(Effect::Choose(options), ctx),
        }
    }

    /// The controller picks a mix of resources from the bank.
    fn gain_choose(
        &mut self,
        ctx: &Ctx,
        to: Target,
        n: Count,
        exclude: &[ResourceId],
        identical: bool,
    ) {
        let units = match n {
            Count::Fixed(k) => k,
            Count::Var => ctx.bound.x,
            Count::CostOf { discarded, bonus } => {
                let card = if discarded {
                    ctx.bound.discarded
                } else {
                    ctx.bound.card
                };
                bonus + card.map_or(0, |c| self.placement_cost_units(c))
            },
        };
        let kinds: Vec<(ResourceId, u32)> = self
            .state
            .bank
            .iter()
            .filter(|(res, _)| !exclude.contains(res))
            .filter_map(|(res, supply)| match supply {
                Supply::Finite(0) => None,
                Supply::Finite(have) => Some((res, have)),
                Supply::Infinite => Some((res, units)),
            })
            .collect();
        let options = if units == 0 {
            Vec::new()
        } else if identical {
            kinds
                .iter()
                .filter(|(_, have)| *have >= units)
                .map(|(res, _)| Effect::Gain {
                    to,
                    res: *res,
                    n: units,
                })
                .collect()
        } else {
            let capped: Vec<_> = kinds.iter().map(|(r, have)| (*r, (*have).min(units))).collect();
            pay::multisets(&capped, units)
                .into_iter()
                .map(|mix| {
                    Effect::Sequence(
                        mix.into_iter().map(|(res, n)| Effect::Gain { to, res, n }).collect(),
                    )
                })
                .collect()
        };
        self.choose_among(options, *ctx);
    }

    /// The controller picks a rival and gains as many `res` as that rival
    /// holds of `counted`.
    fn gain_per_rival(
        &mut self,
        ctx: &Ctx,
        res: ResourceId,
        counted: ResourceId,
    ) -> Result<(), RuleError> {
        let mut options = Vec::new();
        for &rival in self.state.flow.turn_order.order() {
            if rival == ctx.controller {
                continue;
            }
            let held = self.state.player_pool(rival)?.finite(counted).unwrap_or(0);
            if held > 0 {
                options.push(Effect::Gain {
                    to: Target::Controller,
                    res,
                    n: held,
                });
            }
        }
        self.choose_among(options, *ctx);
        Ok(())
    }

    /// Every way `player` could use one of their React abilities of these
    /// kinds right now: `(card, ability index, payment)`.
    fn react_uses(&self, player: PlayerId, kinds: &[ReactKind]) -> Vec<(EntityId, usize, Payment)> {
        let mut out = Vec::new();
        for entity in pay::own_cards(&self.state, player) {
            for (index, a) in abilities_of(&self.state, entity).iter().enumerate() {
                let Trigger::React(kind) = a.trigger else {
                    continue;
                };
                if !kinds.contains(&kind)
                    || !a.condition.holds(&self.state, player, Some(entity))
                    || limit_used(&self.state, entity, index, a.limit)
                {
                    continue;
                }
                for payment in payments(&self.state, player, Some(entity), a.cost.as_ref(), entity)
                {
                    if affordable(&self.state, player, Some(entity), &payment) {
                        out.push((entity, index, payment));
                    }
                }
            }
        }
        out
    }

    /// Offers the controller their React abilities of one kind; without any
    /// (or when they decline) `default` happens.
    fn react_window(&mut self, ctx: &Ctx, kind: ReactKind, default: Effect, repeat: bool) {
        let uses = self.react_uses(ctx.controller, &[kind]);
        if uses.is_empty() {
            self.push_effect(default, *ctx);
            return;
        }
        let again = Effect::ReactWindow {
            kind,
            default: Box::new(default.clone()),
            repeat,
        };
        let mut options = vec![default];
        for (source, ability, payment) in uses {
            let reaction = Effect::Use {
                source,
                ability,
                payment,
            };
            options.push(if repeat {
                Effect::Sequence(vec![reaction, again.clone()])
            } else {
                reaction
            });
        }
        self.push_effect(Effect::Choose(options), *ctx);
    }

    /// One affected player's share of an effect aimed at several: they may
    /// avoid it through a React ability or the effect's own escape; otherwise
    /// `effect` resolves with them as controller.
    fn affect_one(
        &mut self,
        ctx: &Ctx,
        player: PlayerId,
        defense: &Defense,
        effect: Effect,
    ) -> Result<(), RuleError> {
        if defense.skip_passed && self.state.flow.turn_order.has_passed(player) {
            return Ok(());
        }
        let mut options = vec![effect];
        for (source, ability, payment) in self.react_uses(player, &defense.react) {
            options.push(Effect::Use {
                source,
                ability,
                payment,
            });
        }
        options.extend(self.escape_option(player, defense.escape.as_ref())?);
        self.choose_among(options, ctx.for_player(player));
        Ok(())
    }

    /// The effect that avoids something through the effect's own escape, if the
    /// player can use it now.
    fn escape_option(
        &self,
        player: PlayerId,
        escape: Option<&Escape>,
    ) -> Result<Option<Effect>, RuleError> {
        let Some(escape) = escape else {
            return Ok(None);
        };
        let rules = self.state.card_rules.as_ref();
        Ok(match escape {
            Escape::Pay(res, n) => self
                .state
                .player_pool(player)?
                .has(*res, *n)
                .then_some(Effect::Pay { res: *res, n: *n }),
            Escape::DiscardCard => match rules {
                Some(r) => {
                    let hand = self.state.zone_id(&Owner::Player(player), r.hand)?;
                    (!self.state.zone(hand)?.is_empty()).then_some(Effect::DiscardChoose { n: 1 })
                },
                None => None,
            },
            Escape::Destroy(tag) => rules.and_then(|r| {
                pay::own_cards(&self.state, player)
                    .iter()
                    .any(|c| pay::has_tag(&self.state, *c, tag))
                    .then(|| Effect::Pick {
                        from:   r.tableau,
                        owner:  OwnerRef::Controller,
                        filter: Filter {
                            tag: Some(tag.clone()),
                            ..Filter::default()
                        },
                        then:   Box::new(Effect::Destroy {
                            target: Target::Card,
                        }),
                    })
            }),
        })
    }

    /// The controller loses `n` of a resource; each unit they cannot lose
    /// costs them `shortfall` other resources of their choice, as far as they
    /// have them.
    fn lose(
        &mut self,
        ctx: &Ctx,
        res: ResourceId,
        n: u32,
        shortfall: u32,
    ) -> Result<(), RuleError> {
        let player = ctx.controller;
        let lost = self.state.player_pool(player)?.finite(res).unwrap_or(0).min(n);
        if lost > 0 {
            self.state.pay_player(player, res, lost)?;
        }
        let missing = n - lost;
        if missing == 0 {
            return Ok(());
        }
        let others: Vec<(ResourceId, u32)> = self
            .state
            .player_pool(player)?
            .iter()
            .filter(|(other, _)| *other != res)
            .filter_map(|(other, supply)| match supply {
                Supply::Finite(have) if have > 0 => Some((other, have)),
                _ => None,
            })
            .collect();
        let units = (shortfall * missing).min(others.iter().map(|(_, have)| have).sum());
        if units > 0 {
            let options = pay::multisets(&others, units)
                .into_iter()
                .map(|mix| {
                    Effect::Sequence(
                        mix.into_iter().map(|(res, n)| Effect::Pay { res, n }).collect(),
                    )
                })
                .collect();
            self.choose_among(options, *ctx);
        }
        Ok(())
    }

    /// The controller discards `n` cards of their choice from hand, one at a
    /// time; with `n` or fewer cards the whole hand goes.
    fn discard_choose(&mut self, ctx: &Ctx, n: u32) -> Result<(), RuleError> {
        if n == 0 {
            return Ok(());
        }
        let rules = self.state.card_rules.clone().ok_or(RuleError::Unsupported)?;
        let hand = self.state.zone_id(&Owner::Player(ctx.controller), rules.hand)?;
        let cards = self.state.zone(hand)?.entities().to_vec();
        let discard = |card| Effect::WithCard {
            card,
            effect: Box::new(Effect::Move {
                entity: Target::Card,
                to:     rules.discard,
                owner:  OwnerRef::Controller,
            }),
        };
        if cards.len() <= n as usize {
            for card in cards.into_iter().rev() {
                self.push_effect(discard(card), *ctx);
            }
            return Ok(());
        }
        let options = cards
            .into_iter()
            .map(|card| Effect::Sequence(vec![discard(card), Effect::DiscardChoose { n: n - 1 }]))
            .collect();
        self.push_effect(Effect::Choose(options), *ctx);
        Ok(())
    }

    /// The controller picks the order of the top `n` cards of a zone.
    fn reorder(
        &mut self,
        ctx: &Ctx,
        owner: &OwnerRef,
        kind: ZoneKind,
        n: usize,
    ) -> Result<(), RuleError> {
        let zone = self.state.zone_id(&ctx.owner(owner), kind)?;
        // Looking at cards runs the deck dry like drawing does: the discard
        // pile is shuffled in underneath what is left.
        self.state.top_up(zone, n)?;
        let n = n.min(self.state.zone(zone)?.len());
        if n < 2 {
            return Ok(());
        }
        let options = permutations(n)
            .into_iter()
            .map(|perm| Effect::Permute {
                owner: owner.clone(),
                zone: kind,
                perm,
            })
            .collect();
        self.push_effect(Effect::Choose(options), *ctx);
        Ok(())
    }

    fn decide(&mut self, player: PlayerId, choice: usize) -> Result<(), RuleError> {
        let pending = self.state.pending.as_mut().ok_or(RuleError::Illegal)?;
        pending.answers.push((player, choice));
        let complete = pending.answers.len() == pending.players.len();
        self.state.emit(Event::DecisionMade(player));
        if complete && let Some(done) = self.state.pending.take() {
            // The first answer resolves first, so it goes on top of the stack.
            for (who, picked) in done.answers.iter().rev() {
                let effect = done.options.get(*picked).cloned().ok_or(RuleError::Illegal)?;
                let mut ctx = Ctx::new(done.source, *who);
                ctx.bound = done.bound;
                self.push_effect(effect, ctx);
            }
        }
        Ok(())
    }

    fn push_effect(&mut self, effect: Effect, ctx: Ctx) {
        self.state.stack.push(Task::Effect { effect, ctx });
    }

    /// Runs the resolution loop until the stack is empty or a decision is
    /// needed.
    fn settle(&mut self) -> Result<(), RuleError> {
        let mut steps = 0;
        loop {
            while let Some(event) = self.state.pop_event() {
                self.process_event(&event);
            }
            if self.state.pending.is_some() {
                return Ok(());
            }
            if self.state.winners.is_some() {
                self.state.stack.clear();
                return Ok(());
            }
            let Some(task) = self.state.stack.pop() else {
                return Ok(());
            };
            steps += 1;
            if steps > MAX_STEPS {
                return Err(RuleError::Runaway);
            }
            self.run_task(task)?;
        }
    }

    fn process_event(&mut self, event: &Event) {
        self.events.push(event.clone());
        if self.state.winners.is_some() {
            return;
        }
        self.state.modifiers.retain_mut(|m| m.update(event));
        self.state.used.retain(|u| !u.resets_on(event));
        self.fire_triggers(event);
        self.check_game_end(event);
    }

    /// Queues the effects of every ability triggered by the event. Players are
    /// served in turn order starting with the active player; the first one
    /// resolves first.
    fn fire_triggers(&mut self, event: &Event) {
        let kind = event.kind();
        let active = self.state.flow.active_player();
        let order = self.state.flow.turn_order.order();
        let start = active.and_then(|a| order.iter().position(|p| *p == a)).unwrap_or(0);
        let rank = |player: PlayerId| {
            order
                .iter()
                .position(|p| *p == player)
                .map_or(usize::MAX, |i| (i + order.len() - start) % order.len())
        };

        let mut zones: Vec<_> = self
            .state
            .zones()
            .filter(|z| z.abilities_active)
            .filter_map(|z| z.owner.lead().map(|lead| (rank(lead), lead, z.id)))
            .collect();
        zones.sort_by_key(|(rank, _, id)| (*rank, *id));

        let mut fired = Vec::new();
        for (_, controller, zone) in zones {
            let Ok(zone) = self.state.zone(zone) else {
                continue;
            };
            for &entity in zone.entities() {
                for (index, a) in abilities_of(&self.state, entity).iter().enumerate() {
                    if a.trigger == Trigger::OnEvent(kind)
                        && a.condition.holds(&self.state, controller, Some(entity))
                        && !limit_used(&self.state, entity, index, a.limit)
                    {
                        let used = AbilityUse {
                            entity,
                            ability: index,
                            controller,
                            limit: a.limit,
                        };
                        fired.push((a.effect.clone(), Ctx::new(Some(entity), controller), used));
                    }
                }
            }
        }
        for (_, _, used) in &fired {
            if used.limit != Limit::None {
                self.state.used.push(used.clone());
            }
        }
        for (effect, ctx, _) in fired.into_iter().rev() {
            self.push_effect(effect, ctx);
        }
    }

    fn check_game_end(&mut self, event: &Event) {
        let Some(policy) = self.state.flow.game_end.clone() else {
            return;
        };
        let due = match policy.when {
            crate::flow::EndCheck::AfterEvent => true,
            crate::flow::EndCheck::RoundEnd => matches!(event, Event::RoundEnded(_)),
        };
        if !due {
            return;
        }
        let players = self.state.flow.turn_order.order();
        if let Some(winners) = policy.winners(&self.state.counters, players) {
            self.state.winners = Some(winners.clone());
            self.state.emit(Event::GameEnded(winners));
        }
    }

    fn run_task(&mut self, task: Task) -> Result<(), RuleError> {
        match task {
            Task::Effect { effect, ctx } => self.apply_effect(effect, ctx),
            Task::EndTurn => {
                let player = self.state.flow.turn_order.current().ok_or(RuleError::Illegal)?;
                self.state.stack.push(Task::NextTurn);
                self.state.emit(Event::TurnEnded(player));
                for market in self.state.markets.clone() {
                    if market.policy.refill == crate::market::Refill::EndOfTurn {
                        market.refill(&mut self.state)?;
                    }
                }
                let effects = self.rules.turn_end_effects(&self.state, player);
                if !effects.is_empty() {
                    self.push_effect(Effect::Sequence(effects), Ctx::new(None, player));
                }
                Ok(())
            },
            Task::NextTurn => {
                let flow = &mut self.state.flow;
                if flow.phase_over() {
                    self.state.stack.push(Task::NextPhase);
                } else if let Some(next) = flow.turn_order.advance() {
                    flow.actions_taken = 0;
                    self.state.emit(Event::TurnStarted(next));
                } else {
                    self.state.stack.push(Task::NextPhase);
                }
                Ok(())
            },
            Task::NextPhase => self.next_phase(),
        }
    }

    fn next_phase(&mut self) -> Result<(), RuleError> {
        let flow = &mut self.state.flow;
        if flow.phases.is_empty() {
            return Err(RuleError::NoPhases);
        }
        let next = flow.phase.map_or(0, |i| i + 1);
        if next >= flow.phases.len() {
            // The round is over: reorder, then come back for the first phase of
            // the next round.
            let round = flow.round;
            flow.phase = None;
            flow.turn_order.reorder();
            self.state.stack.push(Task::NextPhase);
            self.state.emit(Event::RoundEnded(round));
            return Ok(());
        }
        let started_round = if next == 0 {
            flow.round += 1;
            Some(flow.round)
        } else {
            None
        };
        flow.phase = Some(next);
        let phase = flow.phases[next];
        let first = if phase.interactive {
            flow.turn_order.begin_phase();
            flow.actions_taken = 0;
            flow.turn_order.current()
        } else {
            None
        };
        if let Some(round) = started_round {
            self.state.emit(Event::RoundStarted(round));
        }
        if !phase.interactive {
            self.state.stack.push(Task::NextPhase);
        }
        let effects = self.rules.phase_effects(&self.state, phase.id);
        if !effects.is_empty() {
            let lead = self.state.flow.turn_order.order().first().copied();
            if let Some(lead) = lead {
                self.push_effect(Effect::Sequence(effects), Ctx::new(None, lead));
            }
        }
        self.state.emit(Event::PhaseStarted(phase.id));
        if let Some(player) = first {
            self.state.emit(Event::TurnStarted(player));
        }
        Ok(())
    }

    // One arm per effect; splitting it up is on the plan (plan.md, 8.1).
    #[allow(clippy::too_many_lines)]
    fn apply_effect(&mut self, effect: Effect, ctx: Ctx) -> Result<(), RuleError> {
        match effect {
            Effect::Gain { to, res, n } => {
                match ctx.subject(to).ok_or(RuleError::BadTarget)? {
                    Subject::Player(p) => self.state.gain_player(p, res, n)?,
                    Subject::Entity(e) => self.state.gain_entity(e, res, n)?,
                };
            },
            Effect::Draw { who, from, to, n } => {
                self.state.draw(&ctx.owner(&who), from, to, n)?;
            },
            Effect::Move { entity, to, owner } => {
                let Some(Subject::Entity(entity)) = ctx.subject(entity) else {
                    return Err(RuleError::BadTarget);
                };
                let zone = self.state.zone_id(&ctx.owner(&owner), to)?;
                self.state.move_entity(entity, zone, Pos::Top)?;
            },
            Effect::MoveAll { from, to, owner } => {
                let owner = ctx.owner(&owner);
                let (from, to) = (
                    self.state.zone_id(&owner, from)?,
                    self.state.zone_id(&owner, to)?,
                );
                for entity in self.state.zone(from)?.entities().to_vec() {
                    self.state.move_entity(entity, to, Pos::Top)?;
                }
            },
            Effect::ModifyCounter {
                counter,
                owner,
                delta,
            } => {
                let owner = ctx.owner(&owner);
                self.state
                    .add_to_counter(counter, &owner, delta)
                    .ok_or(RuleError::NoCounter(counter, owner))?;
            },
            Effect::AddModifier {
                target,
                rule,
                duration,
            } => {
                let subject = ctx.subject(target).ok_or(RuleError::BadTarget)?;
                self.state.modifiers.push(Modifier::new(subject, rule, duration));
            },
            Effect::GainX { to, res } => {
                let n = ctx.bound.x;
                match ctx.subject(to).ok_or(RuleError::BadTarget)? {
                    Subject::Player(p) => self.state.gain_player(p, res, n)?,
                    Subject::Entity(e) => self.state.gain_entity(e, res, n)?,
                };
            },
            Effect::GainPaid { to } => {
                let res = ctx.bound.paid.ok_or(RuleError::BadTarget)?;
                match ctx.subject(to).ok_or(RuleError::BadTarget)? {
                    Subject::Player(p) => self.state.gain_player(p, res, 1)?,
                    Subject::Entity(e) => self.state.gain_entity(e, res, 1)?,
                };
            },
            Effect::GainAll { who, res, n } => {
                let order = self.state.flow.turn_order.order().to_vec();
                for p in order {
                    if who == Who::Everyone || p != ctx.controller {
                        self.state.gain_player(p, res, n)?;
                    }
                }
            },
            Effect::Pay { res, n } => self.state.pay_player(ctx.controller, res, n)?,
            Effect::Exhaust(target) => {
                self.state.entity_mut(Self::card_of(&ctx, target)?)?.exhausted = true;
            },
            Effect::Ready(target) => {
                self.state.entity_mut(Self::card_of(&ctx, target)?)?.exhausted = false;
            },
            Effect::ReadyAll { owner } => {
                let owner = ctx.owner(&owner);
                let cards: Vec<_> = self
                    .state
                    .zones()
                    .filter(|z| z.abilities_active && z.owner == owner)
                    .flat_map(|z| z.entities().to_vec())
                    .collect();
                for card in cards {
                    self.state.entity_mut(card)?.exhausted = false;
                }
            },
            Effect::TakeAll { from } => {
                self.state.take_all(Self::card_of(&ctx, from)?, ctx.controller)?;
            },
            Effect::Grow { per_kind } => {
                self.state.grow(ctx.source.ok_or(RuleError::BadTarget)?, per_kind)?;
            },
            Effect::Destroy { target } => self.destroy_card(Self::card_of(&ctx, target)?)?,
            Effect::MoveNth {
                from,
                index,
                to,
                owner,
            } => {
                let owner = ctx.owner(&owner);
                let (from, to) = (
                    self.state.zone_id(&owner, from)?,
                    self.state.zone_id(&owner, to)?,
                );
                let entity =
                    *self.state.zone(from)?.entities().get(index).ok_or(RuleError::BadTarget)?;
                self.state.move_entity(entity, to, Pos::Top)?;
            },
            Effect::ResetCounters { counter } => {
                for c in self.state.counters.iter_mut().filter(|c| c.id == counter) {
                    c.value = c.min;
                }
            },
            Effect::ForEachPlayer(inner) => {
                let order = self.state.flow.turn_order.order().to_vec();
                for p in order.into_iter().rev() {
                    self.push_effect((*inner).clone(), ctx.for_player(p));
                }
            },
            Effect::ForEachOwned { filter, effect } => {
                let mut cards = Vec::new();
                for card in pay::own_cards(&self.state, ctx.controller) {
                    let e = self.state.entity(card)?;
                    let holds = e.pool.iter().any(|(_, s)| s != crate::resource::Supply::Finite(0));
                    let ok = filter.tag.as_ref().is_none_or(|t| pay::has_tag(&self.state, card, t))
                        && filter.exhausted.is_none_or(|x| x == e.exhausted)
                        && filter.has_resources.is_none_or(|h| h == holds);
                    if ok {
                        cards.push(card);
                    }
                }
                for card in cards.into_iter().rev() {
                    self.push_effect(
                        Effect::WithCard {
                            card,
                            effect: effect.clone(),
                        },
                        ctx,
                    );
                }
            },
            Effect::Pick {
                from,
                owner,
                filter,
                then,
            } => {
                let cards = self.candidates(&ctx, from, &owner, &filter)?;
                let options: Vec<_> = cards
                    .into_iter()
                    .map(|card| Effect::WithCard {
                        card,
                        effect: then.clone(),
                    })
                    .collect();
                match options.len() {
                    0 => {},
                    1 => {
                        let only = options.into_iter().next().ok_or(RuleError::BadTarget)?;
                        self.push_effect(only, ctx);
                    },
                    _ => self.push_effect(Effect::Choose(options), ctx),
                }
            },
            Effect::WithCard { card, effect } => self.push_effect(*effect, ctx.with_card(card)),
            Effect::Ignore => {},
            Effect::PlaceCard {
                from,
                tag,
                discount,
            } => {
                self.place_card(&ctx, from, tag.as_ref(), discount.as_ref())?;
            },
            Effect::GainChoose {
                to,
                n,
                exclude,
                identical,
            } => self.gain_choose(&ctx, to, n, &exclude, identical),
            Effect::GainPerRival { res, counted } => self.gain_per_rival(&ctx, res, counted)?,
            Effect::DiscardChoose { n } => self.discard_choose(&ctx, n)?,
            Effect::DrawDiscard { from, to, n } => {
                let drawn = self.state.draw(&Owner::Player(ctx.controller), from, to, n)?;
                self.push_effect(Effect::DiscardChoose { n: drawn }, ctx);
            },
            Effect::Reorder { owner, zone, n } => self.reorder(&ctx, &owner, zone, n)?,
            Effect::Permute { owner, zone, perm } => {
                let zone = self.state.zone_id(&ctx.owner(&owner), zone)?;
                self.state.reorder_top(zone, &perm)?;
            },
            Effect::Use {
                source,
                ability,
                payment,
            } => self.use_ability(ctx.controller, source, ability, &payment)?,
            Effect::Lose { res, n, shortfall } => self.lose(&ctx, res, n, shortfall)?,
            Effect::Affect {
                who,
                defense,
                effect,
            } => {
                // Served in turn order starting after the controller; with
                // `Everyone` the controller comes last.
                let order = self.state.flow.turn_order.order().to_vec();
                let start = order.iter().position(|p| *p == ctx.controller).unwrap_or(0);
                let affected = (1..=order.len())
                    .map(|i| order[(start + i) % order.len()])
                    .filter(|p| who == Who::Everyone || *p != ctx.controller);
                for player in affected.rev() {
                    let one = Effect::AffectOne {
                        player,
                        defense: defense.clone(),
                        effect: effect.clone(),
                    };
                    self.push_effect(one, ctx);
                }
            },
            Effect::AffectOne {
                player,
                defense,
                effect,
            } => self.affect_one(&ctx, player, &defense, *effect)?,
            Effect::ReactWindow {
                kind,
                default,
                repeat,
            } => self.react_window(&ctx, kind, *default, repeat),
            Effect::CheckWin => {
                if let Some(winners) = self.rules.winners(&self.state) {
                    self.state.winners = Some(winners.clone());
                    self.state.emit(Event::GameEnded(winners));
                }
            },
            Effect::Choose(mut options) => {
                options.retain(|o| self.feasible(ctx.controller, o));
                if !options.is_empty() {
                    self.state.emit(Event::DecisionRequested(vec![ctx.controller]));
                    self.state.pending = Some(PendingDecision {
                        players: vec![ctx.controller],
                        options,
                        answers: Vec::new(),
                        source: ctx.source,
                        bound: ctx.bound,
                    });
                }
            },
            Effect::Sequence(effects) => {
                for effect in effects.into_iter().rev() {
                    self.push_effect(effect, ctx);
                }
            },
        }
        Ok(())
    }
}

/// Every ordering of `0..n`, the identity first.
fn permutations(n: usize) -> Vec<Vec<usize>> {
    fn go(rest: &mut Vec<usize>, cur: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
        if rest.is_empty() {
            out.push(cur.clone());
            return;
        }
        for i in 0..rest.len() {
            let x = rest.remove(i);
            cur.push(x);
            go(rest, cur, out);
            cur.pop();
            rest.insert(i, x);
        }
    }
    let mut out = Vec::new();
    go(&mut (0..n).collect(), &mut Vec::new(), &mut out);
    out
}
