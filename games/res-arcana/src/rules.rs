//! The Res Arcana game: table setup, the five phases of a round, the pass
//! procedure and scoring, on top of the generic engine.
//!
//! Everything game specific is data or a composition of the engine's generic
//! effects; the core crate knows nothing about Res Arcana.
//!
//! The optional draft variant ([`ResArcanaRules::with_draft`]) replaces the
//! random starting decks by a draft: see [`DRAFT`].
//!
//! Simplifications (see `plan.md`): the magic items have no "passed side";
//! the places of power are the photographed (first game) sides only; cards
//! that the compiler cannot express are left out of the box.

use std::collections::BTreeMap;

use gamelib::{
    counter::Counter,
    effect::{Bound, Effect, Filter, OwnerRef, PendingDecision, Target, Task},
    engine::{Action, GameRules},
    flow::{Flow, Phase, RoundEndPolicy, TurnOrder, TurnOrderPolicy},
    id::{CounterId, DefId, EntityId, PlayerId, ZoneKind},
    market::{Market, MarketPolicy, Refill},
    rng::Rng,
    state::{CardRules, GameState},
    zone::{Order, Owner, Visibility},
};

use crate::{
    CardKind,
    CardSpec,
    compile::{
        ACTIONS,
        CALM,
        CLEANUP,
        COLLECT,
        COLLECT_STEP,
        Compiled,
        DEATH,
        DECK,
        DISCARD,
        ELAN,
        GOLD,
        HAND,
        LIFE,
        MONUMENT_DECK,
        TABLEAU,
        TAKE,
        TEMP_VP,
        VICTORY,
        VP,
        compile_all,
        resource,
        tag,
        take_from_card,
        victory_check,
    },
};

/// The five places of power, face up in the middle.
pub const PLACES: ZoneKind = ZoneKind(11);
/// The two face-up monuments that can be claimed.
pub const MONUMENT_ROW: ZoneKind = ZoneKind(12);
/// The magic items nobody holds.
pub const ITEMS: ZoneKind = ZoneKind(13);
/// The two mages a player was dealt, before they keep one.
pub const CANDIDATES: ZoneKind = ZoneKind(14);
/// Mages that were not kept.
pub const UNUSED: ZoneKind = ZoneKind(15);
/// The magic item a player is swapping away while they pick the new one.
pub const ASIDE: ZoneKind = ZoneKind(16);

/// The artifacts a player is choosing from in the draft variant: the pack
/// they hold now, passed on around the table after every pick.
pub const DRAFT: ZoneKind = ZoneKind(17);
/// The artifacts dealt to a player in the draft variant that are not in a
/// pack yet.
pub const STOCK: ZoneKind = ZoneKind(18);
/// Artifacts dealt to each player at the start of one draft round.
pub const DRAFT_PACK: u32 = 4;

/// 1 while the player holds the first player token (1 VP at a victory check).
pub const FIRST_TOKEN: CounterId = CounterId(2);

/// Victory points needed at a victory check.
pub const WIN_SCORE: i64 = 10;
/// Artifacts dealt to each player.
pub const DECK_SIZE: usize = 8;
/// Cards in the opening hand.
pub const HAND_SIZE: u32 = 3;
/// The five essences.
pub const ESSENCES: [gamelib::id::ResourceId; 5] = [ELAN, LIFE, CALM, DEATH, GOLD];

/// The rules of Res Arcana (base game).
#[derive(Clone, Debug)]
pub struct ResArcanaRules {
    specs:    Vec<CardSpec>,
    compiled: Compiled,
    draft:    bool,
}

impl ResArcanaRules {
    /// The base game from the embedded card file.
    ///
    /// # Panics
    /// When the embedded card file does not parse (a bug caught by the tests).
    pub fn new() -> Self {
        let specs = crate::load().expect("the embedded card file parses");
        let mut compiled = compile_all(&specs);
        // A rule, not something printed: the essences on any component can be
        // taken during the collect step. Added last, so the indices of the
        // printed abilities stay what the card file says.
        for def in &mut compiled.defs {
            def.abilities.push(take_from_card());
        }
        Self {
            specs,
            compiled,
            draft: false,
        }
    }

    /// The same game with or without the draft variant: instead of being dealt
    /// 8 random artifacts, every player is dealt 4, keeps 1 and passes the
    /// rest to the left, until no artifact is left to pass; then 4 more, kept
    /// the same way but passed to the right. The 8 kept artifacts are
    /// shuffled into the starting deck and the first 3 are drawn. Mages and
    /// magic items are chosen afterwards, as in the normal game.
    #[must_use]
    pub const fn with_draft(mut self, draft: bool) -> Self {
        self.draft = draft;
        self
    }

    /// Whether the draft variant is on.
    pub const fn draft(&self) -> bool {
        self.draft
    }

    /// The compiled cards (ids are positions in the card file).
    pub const fn cards(&self) -> &Compiled {
        &self.compiled
    }

    /// The printed data of a card definition.
    pub fn spec(&self, def: DefId) -> Option<&CardSpec> {
        self.specs.get(usize::try_from(def.0).ok()?)
    }

    /// The definition ids of the compiled cards of one family.
    pub fn defs_of(&self, kind: CardKind) -> Vec<DefId> {
        self.compiled
            .defs
            .iter()
            .filter(|d| self.spec(d.id).is_some_and(|s| s.kind == kind))
            .map(|d| d.id)
            .collect()
    }

    /// A player's score right now: the victory points printed on their cards in
    /// play (plus those counted per essence on the card), points gained by
    /// powers, the first player token and temporary points.
    pub fn score(&self, state: &GameState, player: PlayerId) -> i64 {
        let owner = Owner::Player(player);
        let mut total = 0;
        if let Ok(zone) = state.zone_id(&owner, TABLEAU).and_then(|z| state.zone(z)) {
            for &id in zone.entities() {
                total += self.card_points(state, id);
            }
        }
        for counter in [VP, FIRST_TOKEN, TEMP_VP] {
            total += state.counter(counter, &owner).map_or(0, |c| c.value);
        }
        total
    }

    fn card_points(&self, state: &GameState, id: EntityId) -> i64 {
        let Ok(entity) = state.entity(id) else {
            return 0;
        };
        let Some(spec) = self.spec(entity.def) else {
            return 0;
        };
        let mut points = i64::from(spec.vp.base);
        if let Some((each, essence)) = spec.vp.per_essence {
            let held = entity.pool.finite(resource(essence)).unwrap_or(0);
            points += i64::from(each) * i64::from(held);
        }
        points
    }

    /// Essences in the pool for breaking ties: Gold counts double.
    pub fn tiebreak(state: &GameState, player: PlayerId) -> u32 {
        state.player_pool(player).map_or(0, |pool| {
            ESSENCES
                .iter()
                .map(|r| pool.finite(*r).unwrap_or(0) * if *r == GOLD { 2 } else { 1 })
                .sum()
        })
    }
}

impl Default for ResArcanaRules {
    fn default() -> Self {
        Self::new()
    }
}

/// The reward for discarding a card: 1 Gold or any 2 other essences.
fn discard_reward() -> Effect {
    Effect::Choose(vec![
        Effect::Gain {
            to:  Target::Controller,
            res: GOLD,
            n:   1,
        },
        Effect::GainChoose {
            to:        Target::Controller,
            n:         gamelib::cost::Count::Fixed(2),
            exclude:   vec![GOLD],
            identical: false,
        },
    ])
}

/// Keep the `i`-th artifact of the pack in front of you.
const fn keep_artifact(i: usize) -> Effect {
    Effect::MoveNth {
        from:  DRAFT,
        index: i,
        to:    DECK,
        owner: OwnerRef::Controller,
    }
}

/// Every player's effect, one after the other.
fn each(effect: Effect) -> Effect {
    Effect::ForEachPlayer(Box::new(effect))
}

/// One half of the draft: deal [`DRAFT_PACK`] artifacts to everybody, then
/// everybody keeps one and passes the rest `step` seats along (1 is to the
/// left, -1 to the right) until each pack is down to its last card, which the
/// player who ends up with it keeps.
fn draft_half(step: i32) -> Vec<Effect> {
    let mut steps = vec![each(Effect::Draw {
        who:  OwnerRef::Controller,
        from: STOCK,
        to:   DRAFT,
        n:    DRAFT_PACK,
    })];
    for size in (2..=DRAFT_PACK as usize).rev() {
        steps.push(Effect::Simultaneous((0..size).map(keep_artifact).collect()));
        steps.push(Effect::Rotate { zone: DRAFT, step });
    }
    steps.push(each(Effect::MoveAll {
        from:  DRAFT,
        to:    DECK,
        owner: OwnerRef::Controller,
    }));
    steps
}

/// The whole draft: two halves in opposite directions, then everybody shuffles
/// their 8 artifacts into a deck and draws the opening hand.
fn draft() -> Effect {
    let mut steps = draft_half(1);
    steps.extend(draft_half(-1));
    steps.push(each(Effect::Shuffle {
        owner: OwnerRef::Controller,
        zone:  DECK,
    }));
    steps.push(each(Effect::Draw {
        who:  OwnerRef::Controller,
        from: DECK,
        to:   HAND,
        n:    HAND_SIZE,
    }));
    Effect::Sequence(steps)
}

/// Keep the `i`-th dealt mage, set the other aside.
fn keep_mage(i: usize) -> Effect {
    Effect::Sequence(vec![
        Effect::MoveNth {
            from:  CANDIDATES,
            index: i,
            to:    TABLEAU,
            owner: OwnerRef::Controller,
        },
        Effect::MoveAll {
            from:  CANDIDATES,
            to:    UNUSED,
            owner: OwnerRef::Controller,
        },
    ])
}

fn move_card(to: ZoneKind, owner: OwnerRef) -> Box<Effect> {
    Box::new(Effect::Move {
        entity: Target::Card,
        to,
        owner,
    })
}

fn pick(from: ZoneKind, owner: OwnerRef, tag: Option<&str>, then: Box<Effect>) -> Effect {
    Effect::Pick {
        from,
        owner,
        filter: Filter {
            tag: tag.map(str::to_owned),
            ..Filter::default()
        },
        then,
    }
}

/// Choosing a magic item from the middle.
fn take_item() -> Effect {
    pick(
        ITEMS,
        OwnerRef::Shared,
        None,
        move_card(TABLEAU, OwnerRef::Controller),
    )
}

/// Swap your magic item for a different one: the old one is set aside while
/// the new one is picked, then goes back to the middle.
fn swap_item() -> Effect {
    Effect::Sequence(vec![
        pick(
            TABLEAU,
            OwnerRef::Controller,
            Some(tag::ITEM),
            move_card(ASIDE, OwnerRef::Controller),
        ),
        take_item(),
        pick(
            ASIDE,
            OwnerRef::Controller,
            None,
            move_card(ITEMS, OwnerRef::Shared),
        ),
    ])
}

/// The collect step: every player in turn order does their collect abilities
/// and may take the essences from their components (all of a component or
/// none), in whatever order they like, until they are done.
fn collect_step() -> Effect {
    Effect::ForEachPlayer(Box::new(Effect::ReactWindow {
        kind:    COLLECT_STEP,
        default: Box::new(Effect::Ignore),
        repeat:  true,
    }))
}

/// The collect ability an option of the collect step uses, if it is one that
/// must be done (taking essences off a component is always optional).
fn owed_collect(state: &GameState, option: &Effect) -> bool {
    let used = match option {
        Effect::Sequence(steps) => steps.first(),
        other => Some(other),
    };
    let Some(Effect::Use {
        source, ability, ..
    }) = used
    else {
        return false;
    };
    activated(state, *source, *ability).is_some_and(|(_, a)| {
        a.trigger == gamelib::effect::Trigger::React(COLLECT_STEP)
            && a.effect != take_from_card().effect
    })
}

/// The ability an activation uses, with its card.
fn activated(
    state: &GameState,
    entity: EntityId,
    ability: usize,
) -> Option<(&gamelib::card::Entity, &gamelib::effect::Ability)> {
    let card = state.entity(entity).ok()?;
    let ability = state.defs.get(&card.def)?.abilities.get(ability)?;
    Some((card, ability))
}

/// The "place an artifact" steps inside an effect.
fn placements(effect: &Effect) -> Vec<&Effect> {
    match effect {
        Effect::PlaceCard { .. } => vec![effect],
        Effect::Sequence(steps) | Effect::Choose(steps) => {
            steps.iter().flat_map(placements).collect()
        },
        _ => Vec::new(),
    }
}

/// Whether the player could place a card through this "place an artifact"
/// effect once `payment` is paid.
fn can_place(
    state: &GameState,
    player: PlayerId,
    source: EntityId,
    payment: &gamelib::cost::Payment,
    effect: &Effect,
) -> bool {
    let Effect::PlaceCard {
        from,
        tag: wanted,
        discount,
    } = effect
    else {
        return true;
    };
    // What is left of the pool after the power's own cost.
    let mut after = state.clone();
    for (res, n) in &payment.resources {
        if after.pay_player(player, *res, *n).is_err() {
            return false;
        }
    }
    let gone = |card: EntityId| {
        payment.discarded.contains(&card)
            || payment.destroyed == Some(card)
            || card == source && payment.destroyed == Some(source)
    };
    let owner = Owner::Player(player);
    let Ok(zone) = state.zone_id(&owner, from.unwrap_or(HAND)).and_then(|z| state.zone(z)) else {
        return false;
    };
    zone.entities().iter().any(|card| {
        !gone(*card)
            && wanted.as_ref().is_none_or(|t| gamelib::pay::has_tag(state, *card, t))
            && !gamelib::pay::place_payments(&after, player, *card, discount.as_ref()).is_empty()
    })
}

/// The rulebook's limits on using a power, on top of being able to pay it:
///
/// * the powers of a turned component cannot be used (p.7 and the glossary:
///   "using a power requires its component be straightened"), except a power
///   that only straightens its own component (the Guard Dog, p.10);
/// * a power that places an artifact cannot be used when there is no artifact
///   the player could place with it (a table rule of the owner: nobody pays a
///   cost to place nothing);
/// * the collect step cannot be ended before every collect ability is done.
fn power_allowed(state: &GameState, player: PlayerId, action: &Action) -> bool {
    match action {
        Action::Activate {
            entity,
            ability,
            pay,
        } => {
            let Some((card, used)) = activated(state, *entity, *ability) else {
                return true;
            };
            if card.exhausted && used.effect != Effect::Ready(Target::Source) {
                return false;
            }
            let places = placements(&used.effect);
            if places.is_empty() {
                return true;
            }
            let payments = gamelib::engine::payments(
                state,
                player,
                Some(*entity),
                used.cost.as_ref(),
                *entity,
            );
            let Some(payment) = payments.get(*pay) else {
                return true;
            };
            places.iter().all(|place| can_place(state, player, *entity, payment, place))
        },
        // After paying for such a power the artifact must be placed: declining
        // is not offered.
        Action::Decide(choice) => state.pending.as_ref().is_none_or(|pending| {
            let placing = |o: &Effect| match o {
                Effect::WithCard { effect, .. } => is_placement(effect),
                Effect::Sequence(steps) => matches!(
                    steps.last(),
                    Some(Effect::WithCard { effect, .. }) if is_placement(effect)
                ),
                _ => false,
            };
            let declines = pending.options.get(*choice) == Some(&Effect::Ignore);
            // Nor can the collect step be ended while a collect ability is
            // still to be done.
            let owed = |o: &Effect| placing(o) || owed_collect(state, o);
            !(declines && pending.options.iter().any(owed))
        }),
        _ => true,
    }
}

/// Whether the effect puts the bound card into play.
const fn is_placement(effect: &Effect) -> bool {
    matches!(
        effect,
        Effect::Move {
            entity: Target::Card,
            to:     TABLEAU,
            owner:  OwnerRef::Controller,
        }
    )
}

/// The phases of a round.
fn phases() -> Vec<Phase> {
    [
        (TAKE, false),
        (COLLECT, false),
        (ACTIONS, true),
        (VICTORY, false),
        (CLEANUP, false),
    ]
    .into_iter()
    .map(|(id, interactive)| Phase { id, interactive })
    .collect()
}

impl ResArcanaRules {
    /// A player's zones, counters, starting essences, two dealt mages, their
    /// artifacts and the opening hand.
    fn seat(
        s: &mut GameState,
        p: PlayerId,
        mages: &mut impl Iterator<Item = DefId>,
        artifacts: &mut impl Iterator<Item = DefId>,
        draft: bool,
    ) {
        s.add_player(p);
        let owner = Owner::Player(p);
        for res in ESSENCES {
            // One of each essence to start with; the bank is unlimited.
            let _ = s.gain_player(p, res, 1);
        }
        let deck = s.add_zone(DECK, owner.clone(), Order::Stack, Visibility::Hidden);
        s.add_zone(HAND, owner.clone(), Order::Unordered, Visibility::Owner);
        s.add_zone(DISCARD, owner.clone(), Order::Stack, Visibility::Public);
        let tableau = s.add_zone(TABLEAU, owner.clone(), Order::Unordered, Visibility::Public);
        let candidates = s.add_zone(
            CANDIDATES,
            owner.clone(),
            Order::Unordered,
            Visibility::Owner,
        );
        s.add_zone(UNUSED, owner.clone(), Order::Unordered, Visibility::Hidden);
        s.add_zone(ASIDE, owner.clone(), Order::Unordered, Visibility::Public);
        if let Ok(zone) = s.zone_mut(tableau) {
            zone.abilities_active = true;
        }
        if let Ok(zone) = s.zone_mut(deck) {
            zone.recycle_from = Some(DISCARD);
        }
        for counter in [VP, TEMP_VP, FIRST_TOKEN] {
            s.add_counter(Counter::new(counter, owner.clone(), 0, 0, 1_000));
        }
        for def in mages.take(2) {
            let _ = s.create_entity(def, candidates, true);
        }
        if draft {
            s.add_zone(DRAFT, owner.clone(), Order::Unordered, Visibility::Owner);
            let stock = s.add_zone(STOCK, owner.clone(), Order::Stack, Visibility::Hidden);
            for def in artifacts.take(DECK_SIZE) {
                let _ = s.create_entity(def, stock, true);
            }
            return;
        }
        for def in artifacts.take(DECK_SIZE) {
            let _ = s.create_entity(def, deck, true);
        }
        let _ = s.draw(&owner, DECK, HAND, HAND_SIZE);
    }

    /// The shared part of the table: places, monuments with their market,
    /// and the magic items.
    fn set_table(&self, s: &mut GameState) {
        let shared = |s: &mut GameState, kind, order, visibility| {
            s.add_zone(kind, Owner::Shared, order, visibility)
        };
        let places = shared(s, PLACES, Order::Row, Visibility::Public);
        let monuments = shared(s, MONUMENT_DECK, Order::Stack, Visibility::Hidden);
        let row = shared(s, MONUMENT_ROW, Order::Row, Visibility::Public);
        let items = shared(s, ITEMS, Order::Unordered, Visibility::Public);

        for def in self.defs_of(CardKind::Place) {
            let _ = s.create_entity(def, places, true);
        }
        let mut monument_defs = self.defs_of(CardKind::Monument);
        s.rng.shuffle(&mut monument_defs);
        for def in monument_defs {
            let _ = s.create_entity(def, monuments, true);
        }
        for def in self.defs_of(CardKind::Item) {
            let _ = s.create_entity(def, items, true);
        }

        // Monuments: two face up, or the top of the deck unseen. Places: all
        // five.
        let policy = MarketPolicy {
            top_only:       false,
            refill:         Refill::Immediate,
            removal:        None,
            source_buyable: true,
        };
        let monument_market = Market {
            zone: row,
            source: Some(monuments),
            size: 2,
            dest: TABLEAU,
            policy,
        };
        let _ = monument_market.refill(s);
        s.markets.push(monument_market);
        s.markets.push(Market {
            zone:   places,
            source: None,
            size:   5,
            dest:   TABLEAU,
            policy: MarketPolicy {
                source_buyable: false,
                ..policy
            },
        });
    }
}

impl GameRules for ResArcanaRules {
    fn setup(&self, players: &[PlayerId], rng: Rng) -> GameState {
        let mut s = GameState::new(rng);
        for res in ESSENCES {
            s.bank.set_infinite(res);
        }
        for def in &self.compiled.defs {
            s.add_def(def.clone());
        }
        s.card_rules = Some(CardRules {
            hand:       HAND,
            tableau:    TABLEAU,
            discard:    DISCARD,
            on_discard: Some(discard_reward()),
        });
        self.set_table(&mut s);

        let mut artifacts = self.defs_of(CardKind::Artifact);
        let mut mages = self.defs_of(CardKind::Mage);
        s.rng.shuffle(&mut artifacts);
        s.rng.shuffle(&mut mages);
        let (mut dealt_artifacts, mut dealt_mages) = (artifacts.into_iter(), mages.into_iter());
        for &p in players {
            Self::seat(
                &mut s,
                p,
                &mut dealt_mages,
                &mut dealt_artifacts,
                self.draft,
            );
        }

        s.flow = Flow {
            phases: phases(),
            turn_order: TurnOrder::new(players.to_vec(), TurnOrderPolicy::FirstPasserStarts),
            round_end: RoundEndPolicy::AllPassed,
            actions_per_turn: Some(1),
            ..Flow::default()
        };

        // Everybody keeps one of their two mages, at the same time (after the
        // draft, when there is one).
        let mages_options = vec![keep_mage(0), keep_mage(1)];
        if !self.draft {
            s.pending = Some(PendingDecision {
                players: players.to_vec(),
                options: mages_options.clone(),
                answers: Vec::new(),
                source:  None,
                bound:   Bound::default(),
            });
        }
        // Then the magic items are picked in reverse turn order. The stack
        // resolves its top first, so the first player's pick is pushed first.
        for &p in players {
            s.stack.push(Task::Effect {
                effect: take_item(),
                ctx:    gamelib::effect::Ctx::new(None, p),
            });
        }
        if self.draft {
            // On top of the item picks, so it resolves first: the draft, then
            // the mages.
            for effect in [Effect::Simultaneous(mages_options), draft()] {
                s.stack.push(Task::Effect {
                    effect,
                    ctx: gamelib::effect::Ctx::new(None, players[0]),
                });
            }
        }
        s
    }

    fn legal_actions(&self, state: &GameState, player: PlayerId) -> Vec<Action> {
        // A turn is one action: there is no "do nothing and end the turn".
        gamelib::engine::standard_legal_actions(state, player)
            .into_iter()
            .filter(|a| *a != Action::EndTurn && power_allowed(state, player, a))
            .collect()
    }

    fn phase_effects(&self, _state: &GameState, phase: gamelib::id::PhaseId) -> Vec<Effect> {
        match phase {
            TAKE => vec![collect_step()],
            VICTORY => vec![victory_check(TEMP_VP)],
            CLEANUP => vec![Effect::ForEachPlayer(Box::new(Effect::ReadyAll {
                owner: OwnerRef::Controller,
            }))],
            _ => Vec::new(),
        }
    }

    fn pass_effects(&self, state: &GameState, player: PlayerId) -> Vec<Effect> {
        let mut effects = Vec::new();
        if state.flow.turn_order.passed().first() == Some(&player) {
            // The first to pass takes the token (and with it 1 VP).
            effects.push(Effect::ResetCounters {
                counter: FIRST_TOKEN,
            });
            effects.push(Effect::ModifyCounter {
                counter: FIRST_TOKEN,
                owner:   OwnerRef::Controller,
                delta:   1,
            });
        }
        effects.push(swap_item());
        effects.push(Effect::Draw {
            who:  OwnerRef::Controller,
            from: DECK,
            to:   HAND,
            n:    1,
        });
        effects
    }

    fn winners(&self, state: &GameState) -> Option<Vec<PlayerId>> {
        let order = state.flow.turn_order.order();
        let key = |p: PlayerId| (self.score(state, p), Self::tiebreak(state, p));
        let best = order.iter().map(|p| key(*p)).filter(|(score, _)| *score >= WIN_SCORE).max()?;
        Some(order.iter().copied().filter(|p| key(*p) == best).collect())
    }
}

/// Scores of every player, for logs and bots.
pub fn scoreboard(rules: &ResArcanaRules, state: &GameState) -> BTreeMap<PlayerId, i64> {
    state.flow.turn_order.order().iter().map(|p| (*p, rules.score(state, *p))).collect()
}
