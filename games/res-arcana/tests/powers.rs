//! Plays every power, collect ability and reaction of the 73 base cards once,
//! in a controlled situation, through the real engine and the real
//! `ResArcanaRules`, and checks the printed effect.
//!
//! The harness (`Table`) is a second `GameRules` that delegates everything to
//! `ResArcanaRules`, but after the real setup it replaces the dealt cards by an
//! explicit layout (cards in each tableau, essences on cards, pools, hands,
//! decks, discard piles, the monument row). Every test is then deterministic.
//! The setup decisions (keep a mage, pick a magic item) are covered by
//! `round.rs`; here they are replaced by the layout, and everything after the
//! setup goes through `Engine::apply` with the actions the engine offers.
//!
//! Every one of the 73 base cards has a test named after it: placement or
//! claiming cost, each collect ability, power and reaction (cost paid, effect
//! applied, turning, straightening), and the printed points. `ALL_CARDS` and
//! `ARTIFACT_COSTS` are checked against the card file, so a card that is added
//! or renamed there fails these tables (that a new card also gets a power test
//! is by convention).
//!
//! Tests marked `#[ignore = "bug: ..."]` assert the printed behavior of a card
//! that the engine gets wrong today: Divination with a short deck and the two "2 essences + 1 Life on the card" powers
//! (Chalice of Life, Fountain of Youth).
//!
//! Not covered: the back sides of the places of power and the magic items'
//! "passed" side (not in the game), and the three or four player turn order
//! beyond choosing a rival and hitting every rival.

use std::{collections::BTreeMap, sync::LazyLock};

use gamelib::{
    cost::Payment,
    effect::{Effect, Target, Trigger},
    engine::{self, Action, Engine, GameRules},
    event::Event,
    flow::Phase,
    id::{EntityId, PlayerId, ResourceId, ZoneKind},
    pay,
    rng::Rng,
    state::GameState,
    zone::{Owner, Pos},
};
use gamelib_res_arcana::{
    CardKind,
    compile::{
        ACTIONS,
        CALM,
        COLLECT_STEP,
        DEATH,
        DECK,
        DISCARD,
        ELAN,
        GOLD,
        HAND,
        LIFE,
        MONUMENT_DECK,
        TABLEAU,
        TEMP_VP,
        tag,
    },
    load,
    rules::{
        CANDIDATES,
        ESSENCES,
        FIRST_TOKEN,
        ITEMS,
        MONUMENT_ROW,
        PLACES,
        ResArcanaRules,
        UNUSED,
    },
};

const P0: PlayerId = PlayerId(0);
const P1: PlayerId = PlayerId(1);
const P2: PlayerId = PlayerId(2);

const E: ResourceId = ELAN;
const L: ResourceId = LIFE;
const C: ResourceId = CALM;
const D: ResourceId = DEATH;
const G: ResourceId = GOLD;

/// An amount of one essence.
type Qty = (ResourceId, u32);

fn shared_rules() -> ResArcanaRules {
    static RULES: LazyLock<ResArcanaRules> =
        LazyLock::new(|| ResArcanaRules::new().with_random_start(false));
    RULES.clone()
}

// ---------------------------------------------------------------------------------------------------
// The table: the real setup with an explicit layout
// ---------------------------------------------------------------------------------------------------

/// One component put in a tableau.
#[derive(Clone)]
struct Piece {
    name:    String,
    turned:  bool,
    on_card: Vec<Qty>,
}

/// What one player has.
#[derive(Clone, Default)]
struct Seat {
    pool:    Vec<Qty>,
    tableau: Vec<Piece>,
    hand:    Vec<String>,
    /// Top of the deck first.
    deck:    Vec<String>,
    /// Top of the pile first.
    discard: Vec<String>,
}

/// The game rules of the tests: `ResArcanaRules` plus a layout.
#[derive(Clone)]
struct Table {
    rules:      ResArcanaRules,
    seats:      Vec<Seat>,
    full_round: bool,
    row:        Vec<String>,
}

type Game = Engine<Table>;

fn to_strings(list: &[&str]) -> Vec<String> {
    list.iter().map(ToString::to_string).collect()
}

impl Table {
    /// Only the actions phase: the game starts with player 0 to act.
    fn actions() -> Self {
        Self {
            rules:      shared_rules(),
            seats:      vec![Seat::default(), Seat::default()],
            full_round: false,
            row:        Vec::new(),
        }
    }

    /// The real five phases: take, collect, actions, victory check, cleanup.
    fn round() -> Self {
        Self {
            full_round: true,
            ..Self::actions()
        }
    }

    /// The game has this many players (two by default).
    fn players(mut self, n: usize) -> Self {
        self.seats.resize(n, Seat::default());
        self
    }

    fn seat(&mut self, p: PlayerId) -> &mut Seat {
        &mut self.seats[usize::from(p.0)]
    }

    /// A straightened component in the tableau.
    fn card(self, p: PlayerId, name: &str) -> Self {
        self.piece(p, name, false, &[])
    }

    /// A turned component.
    fn turned(self, p: PlayerId, name: &str) -> Self {
        self.piece(p, name, true, &[])
    }

    /// A straightened component with essences on it.
    fn loaded(self, p: PlayerId, name: &str, on_card: &[Qty]) -> Self {
        self.piece(p, name, false, on_card)
    }

    fn piece(mut self, p: PlayerId, name: &str, turned: bool, on_card: &[Qty]) -> Self {
        self.seat(p).tableau.push(Piece {
            name: name.to_owned(),
            turned,
            on_card: on_card.to_vec(),
        });
        self
    }

    /// The essence pool (nothing else).
    fn pool(mut self, p: PlayerId, pool: &[Qty]) -> Self {
        self.seat(p).pool = pool.to_vec();
        self
    }

    fn hand(mut self, p: PlayerId, names: &[&str]) -> Self {
        self.seat(p).hand = to_strings(names);
        self
    }

    /// The artifact deck, top card first.
    fn deck(mut self, p: PlayerId, names: &[&str]) -> Self {
        self.seat(p).deck = to_strings(names);
        self
    }

    /// The discard pile, top card first.
    fn discard(mut self, p: PlayerId, names: &[&str]) -> Self {
        self.seat(p).discard = to_strings(names);
        self
    }

    /// Monuments that are face up in the middle (the others go back to the
    /// monument deck).
    fn row(mut self, names: &[&str]) -> Self {
        self.row = to_strings(names);
        self
    }

    fn start(self) -> Game {
        let players = u8::try_from(self.seats.len()).expect("a few players");
        let mut e = Engine::new(self, players, 1).expect("the table sets up");
        collect(&mut e);
        e
    }

    fn def(&self, name: &str) -> gamelib::id::DefId {
        self.rules.cards().id_of(name).unwrap_or_else(|| panic!("no card {name}"))
    }

    fn fill(&self, s: &mut GameState, p: PlayerId, kind: ZoneKind, names: &[String]) {
        let zone = s.zone_id(&Owner::Player(p), kind).unwrap();
        // The last card created is the top one.
        for name in names.iter().rev() {
            s.create_entity(self.def(name), zone, false).unwrap();
        }
    }

    fn dress(&self, s: &mut GameState, p: PlayerId) {
        let me = Owner::Player(p);
        let unused = s.zone_id(&me, UNUSED).unwrap();
        for kind in [CANDIDATES, DECK, HAND] {
            let zone = s.zone_id(&me, kind).unwrap();
            for id in s.zone(zone).unwrap().entities().to_vec() {
                s.move_entity(id, unused, Pos::Top).unwrap();
            }
        }
        for res in ESSENCES {
            let have = s.player_pool(p).unwrap().finite(res).unwrap_or(0);
            s.pay_player(p, res, have).unwrap();
        }
        let seat = &self.seats[usize::from(p.0)];
        for &(res, n) in &seat.pool {
            s.gain_player(p, res, n).unwrap();
        }
        let tableau = s.zone_id(&me, TABLEAU).unwrap();
        for piece in &seat.tableau {
            let id = s.create_entity(self.def(&piece.name), tableau, true).unwrap();
            s.entity_mut(id).unwrap().exhausted = piece.turned;
            for &(res, n) in &piece.on_card {
                s.gain_entity(id, res, n).unwrap();
            }
        }
        self.fill(s, p, HAND, &seat.hand);
        self.fill(s, p, DECK, &seat.deck);
        self.fill(s, p, DISCARD, &seat.discard);
    }

    fn arrange_row(&self, s: &mut GameState) {
        if self.row.is_empty() {
            return;
        }
        let row = s.zone_id(&Owner::Shared, MONUMENT_ROW).unwrap();
        let deck = s.zone_id(&Owner::Shared, MONUMENT_DECK).unwrap();
        for id in s.zone(row).unwrap().entities().to_vec() {
            s.move_entity(id, deck, Pos::Bottom).unwrap();
        }
        for name in &self.row {
            let def = self.def(name);
            let id = s
                .entities()
                .find(|x| x.def == def && s.zone(x.zone).is_ok_and(|z| z.owner == Owner::Shared))
                .map(|x| x.id)
                .expect("a monument in the middle");
            s.move_entity(id, row, Pos::Top).unwrap();
        }
    }
}

impl GameRules for Table {
    fn setup(&self, players: &[PlayerId], rng: Rng) -> GameState {
        let mut s = self.rules.setup(players, rng);
        // The setup decisions (keep a mage, pick an item) are replaced by the
        // layout.
        s.pending = None;
        s.stack.clear();
        if !self.full_round {
            s.flow.phases = vec![Phase {
                id:          ACTIONS,
                interactive: true,
            }];
        }
        for &p in players {
            self.dress(&mut s, p);
        }
        self.arrange_row(&mut s);
        s
    }

    fn legal_actions(&self, state: &GameState, player: PlayerId) -> Vec<Action> {
        self.rules.legal_actions(state, player)
    }

    fn winners(&self, state: &GameState) -> Option<Vec<PlayerId>> {
        self.rules.winners(state)
    }

    fn phase_effects(&self, state: &GameState, phase: gamelib::id::PhaseId) -> Vec<Effect> {
        self.rules.phase_effects(state, phase)
    }

    fn pass_effects(&self, state: &GameState, player: PlayerId) -> Vec<Effect> {
        self.rules.pass_effects(state, player)
    }
}

// ---------------------------------------------------------------------------------------------------
// Reading the game
// ---------------------------------------------------------------------------------------------------

fn pool(e: &Game, p: PlayerId, res: ResourceId) -> u32 {
    e.state().player_pool(p).unwrap().finite(res).unwrap_or(0)
}

/// The nonzero part of a list of amounts, as a map.
fn amounts(list: &[Qty]) -> BTreeMap<ResourceId, u32> {
    let mut map = BTreeMap::new();
    for &(res, n) in list {
        *map.entry(res).or_insert(0) += n;
    }
    map.retain(|_, n| *n > 0);
    map
}

fn pool_map(e: &Game, p: PlayerId) -> BTreeMap<ResourceId, u32> {
    let all: Vec<Qty> = ESSENCES.iter().map(|r| (*r, pool(e, p, *r))).collect();
    amounts(&all)
}

/// The player's pool is exactly these essences.
#[track_caller]
fn assert_pool(e: &Game, p: PlayerId, expected: &[Qty]) {
    assert_eq!(pool_map(e, p), amounts(expected), "pool of {p:?}");
}

fn def_name(e: &Game, id: EntityId) -> String {
    let def = e.state().entity(id).unwrap().def;
    e.state().defs[&def].name.clone()
}

fn zone_cards(e: &Game, owner: &Owner, kind: ZoneKind) -> Vec<EntityId> {
    let zone = e.state().zone_id(owner, kind).unwrap();
    e.state().zone(zone).unwrap().entities().to_vec()
}

/// Names of the cards of a zone, sorted.
fn names(e: &Game, p: PlayerId, kind: ZoneKind) -> Vec<String> {
    let mut all: Vec<String> =
        zone_cards(e, &Owner::Player(p), kind).iter().map(|id| def_name(e, *id)).collect();
    all.sort();
    all
}

/// Names of the top `n` cards of a zone, top first.
fn top_names(e: &Game, owner: &Owner, kind: ZoneKind, n: usize) -> Vec<String> {
    zone_cards(e, owner, kind).iter().rev().take(n).map(|id| def_name(e, *id)).collect()
}

fn has(e: &Game, p: PlayerId, kind: ZoneKind, name: &str) -> bool {
    names(e, p, kind).iter().any(|n| n == name)
}

/// A card of the player in the given zone.
#[track_caller]
fn find(e: &Game, p: PlayerId, kind: ZoneKind, name: &str) -> EntityId {
    zone_cards(e, &Owner::Player(p), kind)
        .into_iter()
        .find(|id| def_name(e, *id) == name)
        .unwrap_or_else(|| panic!("{name} is not in the {kind:?} of {p:?}"))
}

/// A component in the player's tableau.
#[track_caller]
fn card(e: &Game, p: PlayerId, name: &str) -> EntityId {
    find(e, p, TABLEAU, name)
}

fn on_card(e: &Game, id: EntityId, res: ResourceId) -> u32 {
    e.state().entity(id).unwrap().pool.finite(res).unwrap_or(0)
}

fn is_turned(e: &Game, id: EntityId) -> bool {
    e.state().entity(id).unwrap().exhausted
}

fn score(e: &Game, p: PlayerId) -> i64 {
    e.rules().rules.score(e.state(), p)
}

fn temp_points(e: &Game, p: PlayerId) -> i64 {
    e.state().counter(TEMP_VP, &Owner::Player(p)).unwrap().value
}

fn first_token(e: &Game, p: PlayerId) -> i64 {
    e.state().counter(FIRST_TOKEN, &Owner::Player(p)).unwrap().value
}

// ---------------------------------------------------------------------------------------------------
// Doing things
// ---------------------------------------------------------------------------------------------------

#[track_caller]
fn decide(e: &mut Game, p: PlayerId, option: usize) {
    e.apply(p, Action::Decide(option)).expect("the decision applies");
}

fn options(e: &Game) -> &[Effect] {
    &e.pending().expect("a decision is pending").options
}

#[track_caller]
fn decide_where(e: &mut Game, p: PlayerId, wanted: impl Fn(&Effect) -> bool) {
    let at = options(e).iter().position(wanted).unwrap_or_else(|| {
        panic!("no such option among {:#?}", options(e));
    });
    decide(e, p, at);
}

/// The essences an option gives, when it only gives essences to the player or
/// to the card.
fn gains(effect: &Effect) -> Option<BTreeMap<ResourceId, u32>> {
    sum_leaves(effect, &|leaf| match leaf {
        Effect::Gain { res, n, .. } => Some((*res, *n)),
        _ => None,
    })
}

/// Adds up the amounts of the leaves of an effect (itself or the steps of a
/// sequence); `None` when some step is not a leaf `amount` understands.
fn sum_leaves(
    effect: &Effect,
    amount: &dyn Fn(&Effect) -> Option<Qty>,
) -> Option<BTreeMap<ResourceId, u32>> {
    let mut all = BTreeMap::new();
    let steps = match effect {
        Effect::Sequence(steps) => steps.as_slice(),
        leaf => std::slice::from_ref(leaf),
    };
    for step in steps {
        let (res, n) = amount(step)?;
        *all.entry(res).or_insert(0) += n;
    }
    Some(all)
}

/// Every option gives essences; the sorted list of what they give.
fn gain_options(e: &Game) -> Vec<BTreeMap<ResourceId, u32>> {
    let mut all: Vec<_> = options(e)
        .iter()
        .map(|o| gains(o).unwrap_or_else(|| panic!("not a gain: {o:?}")))
        .collect();
    all.sort();
    all
}

#[track_caller]
fn decide_gain(e: &mut Game, p: PlayerId, wanted: &[Qty]) {
    let wanted = amounts(wanted);
    decide_where(e, p, |o| gains(o) == Some(wanted.clone()));
}

/// Whether the option of a collect step uses this kind of step: a collect
/// ability (`take == false`) or taking the essences off a component.
fn collect_option(e: &Game, option: &Effect, take: bool) -> Option<EntityId> {
    let first = match option {
        Effect::Sequence(steps) => steps.first()?,
        other => other,
    };
    let Effect::Use {
        source, ability, ..
    } = first
    else {
        return None;
    };
    let def = e.state().entity(*source).ok()?.def;
    let used = e.state().defs[&def].abilities.get(*ability)?;
    let is_take = matches!(used.effect, Effect::TakeAll { .. });
    (used.trigger == Trigger::React(COLLECT_STEP) && is_take == take).then_some(*source)
}

/// Goes on with the collect step the way the tests expect it: every collect
/// ability is done as soon as it is offered (the first one each time), and a
/// step with nothing left to do is ended. It stops at a real choice: what a
/// collect ability gives, or whether to take the essences off a component.
fn collect(e: &mut Game) {
    while let Some(pending) = e.pending() {
        if pending.options.first() != Some(&Effect::Ignore) {
            return;
        }
        let who = pending.players[0];
        let owed = pending.options.iter().position(|o| collect_option(e, o, false).is_some());
        let takes = pending.options.iter().any(|o| collect_option(e, o, true).is_some());
        match owed {
            Some(at) => decide(e, who, at),
            None if takes => return,
            None if pending.options.len() == 1 => decide(e, who, 0),
            None => return,
        }
    }
}

/// In the collect step: take the essences off this component.
#[track_caller]
fn take_from(e: &mut Game, p: PlayerId, id: EntityId) {
    decide_where(e, p, |o| collect_option_source(o) == Some(id));
    collect(e);
}

/// In the collect step: leave the essences on the components and go on.
#[track_caller]
fn leave_essences(e: &mut Game, p: PlayerId) {
    decide_where(e, p, |o| *o == Effect::Ignore);
    collect(e);
}

/// The component an option of a collect step is about.
fn collect_option_source(option: &Effect) -> Option<EntityId> {
    let first = match option {
        Effect::Sequence(steps) => steps.first()?,
        other => other,
    };
    match first {
        Effect::Use { source, .. } => Some(*source),
        _ => None,
    }
}

/// Answers every pending decision with its first option.
fn settle(e: &mut Game) {
    for _ in 0..100 {
        let Some(pending) = e.pending() else {
            return;
        };
        let who =
            *pending.players.iter().find(|p| pending.waiting_for(**p)).expect("someone waits");
        decide(e, who, 0);
    }
    panic!("decisions never ended");
}

/// The number of printed abilities of a card (the rule that lets a player
/// take the essences off any component is not one of them).
fn abilities_of(e: &Game, id: EntityId) -> usize {
    let def = e.state().entity(id).unwrap().def;
    e.state().defs[&def]
        .abilities
        .iter()
        .filter(|a| !matches!(a.effect, Effect::TakeAll { .. }))
        .count()
}

/// Index among the card's abilities of its `nth` ability with this trigger.
fn ability_of(e: &Game, id: EntityId, trigger: Trigger, nth: usize) -> usize {
    let def = e.state().entity(id).unwrap().def;
    e.state().defs[&def]
        .abilities
        .iter()
        .enumerate()
        .filter(|(_, a)| a.trigger == trigger)
        .nth(nth)
        .map(|(i, _)| i)
        .unwrap_or_else(|| panic!("{} has no {trigger:?} #{nth}", def_name(e, id)))
}

/// Every way `p` could pay for the `nth` power of the card now, with the index
/// the action needs.
fn power_payments(e: &Game, p: PlayerId, id: EntityId, nth: usize) -> Vec<(usize, Payment)> {
    let ability = ability_of(e, id, Trigger::Activated, nth);
    let def = e.state().entity(id).unwrap().def;
    let cost = e.state().defs[&def].abilities[ability].cost.clone();
    let all = engine::payments(e.state(), p, Some(id), cost.as_ref(), id);
    e.legal_actions(p)
        .into_iter()
        .filter_map(|a| match a {
            Action::Activate {
                entity,
                ability: i,
                pay,
            } if entity == id && i == ability => Some((pay, all[pay].clone())),
            _ => None,
        })
        .collect()
}

fn can_use(e: &Game, p: PlayerId, id: EntityId, nth: usize) -> bool {
    !power_payments(e, p, id, nth).is_empty()
}

#[track_caller]
fn use_power_where(
    e: &mut Game,
    p: PlayerId,
    id: EntityId,
    nth: usize,
    wanted: impl Fn(&Payment) -> bool,
) {
    let ability = ability_of(e, id, Trigger::Activated, nth);
    let choices = power_payments(e, p, id, nth);
    let (pay, _) = choices
        .iter()
        .find(|(_, payment)| wanted(payment))
        .unwrap_or_else(|| panic!("no such way to pay among {choices:#?}"));
    let action = Action::Activate {
        entity: id,
        ability,
        pay: *pay,
    };
    e.apply(p, action).expect("the power resolves");
}

/// Uses the `nth` power when there is exactly one way to pay for it.
#[track_caller]
fn use_power(e: &mut Game, p: PlayerId, id: EntityId, nth: usize) {
    assert_eq!(
        power_payments(e, p, id, nth).len(),
        1,
        "exactly one way to pay"
    );
    use_power_where(e, p, id, nth, |_| true);
}

/// Uses the `nth` power paying exactly these essences from the pool.
#[track_caller]
fn use_power_paying(e: &mut Game, p: PlayerId, id: EntityId, nth: usize, paid: &[Qty]) {
    let paid = amounts(paid);
    use_power_where(e, p, id, nth, |payment| payment.resources == paid);
}

/// The ways of placing the card (in hand), with the index the action needs.
fn place_payments(e: &Game, p: PlayerId, id: EntityId) -> Vec<(usize, Payment)> {
    let all = pay::place_payments(e.state(), p, id, None);
    e.legal_actions(p)
        .into_iter()
        .filter_map(|a| match a {
            Action::Place { entity, pay } if entity == id => Some((pay, all[pay].clone())),
            _ => None,
        })
        .collect()
}

/// Whether the card in `p`'s hand can be placed at all now.
fn can_place(e: &Game, p: PlayerId, name: &str) -> bool {
    !place_payments(e, p, find(e, p, HAND, name)).is_empty()
}

/// Places the card from hand, paying exactly these essences.
#[track_caller]
fn place(e: &mut Game, p: PlayerId, name: &str, paid: &[Qty]) {
    let id = find(e, p, HAND, name);
    let paid = amounts(paid);
    let choices = place_payments(e, p, id);
    let (pay, _) = choices
        .iter()
        .find(|(_, payment)| payment.resources == paid)
        .unwrap_or_else(|| panic!("cannot place {name} for {paid:?}; the ways are {choices:?}"));
    e.apply(
        p,
        Action::Place {
            entity: id,
            pay:    *pay,
        },
    )
    .expect("the card is placed");
}

/// A card in the middle (monument row, monument deck or places of power).
fn shared_card(e: &Game, name: &str) -> EntityId {
    let def = e.rules().def(name);
    e.state()
        .entities()
        .find(|x| x.def == def && e.state().zone(x.zone).is_ok_and(|z| z.owner == Owner::Shared))
        .unwrap_or_else(|| panic!("{name} is not in the middle"))
        .id
}

/// The ways of paying to claim a card from the middle, with action indexes.
fn buy_payments(e: &Game, p: PlayerId, id: EntityId) -> Vec<(usize, Payment)> {
    let def = e.state().entity(id).unwrap().def;
    let cost = e.state().defs[&def].cost.clone();
    let all = engine::payments(e.state(), p, None, cost.as_ref(), id);
    e.legal_actions(p)
        .into_iter()
        .filter_map(|a| match a {
            Action::Buy { entity, pay } if entity == id => Some((pay, all[pay].clone())),
            _ => None,
        })
        .collect()
}

fn can_claim(e: &Game, p: PlayerId, name: &str) -> bool {
    !buy_payments(e, p, shared_card(e, name)).is_empty()
}

/// Claims a monument or place of power from the middle, paying exactly this.
#[track_caller]
fn claim(e: &mut Game, p: PlayerId, name: &str, paid: &[Qty]) {
    let id = shared_card(e, name);
    let paid = amounts(paid);
    let choices = buy_payments(e, p, id);
    let (pay, _) = choices
        .iter()
        .find(|(_, payment)| payment.resources == paid)
        .unwrap_or_else(|| panic!("cannot claim {name} for {paid:?}; the ways are {choices:?}"));
    e.apply(
        p,
        Action::Buy {
            entity: id,
            pay:    *pay,
        },
    )
    .expect("the card is claimed");
}

/// Whether the pending decision is picking a magic item from the middle.
fn picks_an_item(e: &Game) -> bool {
    e.pending().is_some_and(|pending| {
        !pending.options.is_empty()
            && pending.options.iter().all(|o| {
                matches!(o, Effect::WithCard { card, .. }
                if e.state().entity(*card).is_ok_and(|x| {
                    e.state().zone(x.zone).is_ok_and(|z| z.kind == ITEMS)
                }))
            })
    })
}

/// Player `p` passes and picks whatever magic item is offered first (other
/// decisions, such as a victory check reaction, stay pending).
fn pass(e: &mut Game, p: PlayerId) {
    e.apply(p, Action::Pass).expect("passing is always possible");
    while picks_an_item(e) {
        decide(e, p, 0);
    }
    // When that ended the round, the next one starts with its collect step.
    collect(e);
}

/// Player 0 uses up its action by discarding the first card of its hand, then
/// player 1 passes: afterwards it is player 0's turn again and player 1 has
/// passed. Player 0 needs a card in hand and the first option of the discard
/// reward must be the Gold.
fn rival_passes(e: &mut Game) {
    assert_eq!(e.current_player(), Some(P0));
    let spare = zone_cards(e, &Owner::Player(P0), HAND)[0];
    e.apply(P0, Action::Discard { entity: spare }).expect("discarding is possible");
    settle(e);
    assert_eq!(e.current_player(), Some(P1));
    pass(e, P1);
    assert_eq!(e.current_player(), Some(P0));
}

/// Player 1 does something harmless (discards its first card) so that player 0
/// acts again.
fn rival_acts(e: &mut Game) {
    assert_eq!(e.current_player(), Some(P1));
    let spare = zone_cards(e, &Owner::Player(P1), HAND)[0];
    e.apply(P1, Action::Discard { entity: spare }).expect("discarding is possible");
    settle(e);
    assert_eq!(e.current_player(), Some(P0));
}

// ---------------------------------------------------------------------------------------------------
// Shared scenarios
// ---------------------------------------------------------------------------------------------------

/// A collect ability that gives fixed essences without any decision, played in
/// the real collect phase of the first round.
#[track_caller]
fn assert_collects(name: &str, expected: &[Qty]) {
    let e = Table::round().card(P0, name).start();
    assert!(e.pending().is_none(), "{name} collects without a decision");
    assert_pool(&e, P0, expected);
    assert_eq!(
        e.current_player(),
        Some(P0),
        "the round went on to the actions"
    );
}

/// A collect ability that offers a choice: these are the choices, and picking
/// one gives exactly it.
#[track_caller]
fn assert_collect_choice(name: &str, choices: &[&[Qty]], pick: usize) {
    let mut e = Table::round().card(P0, name).start();
    let mut wanted: Vec<_> = choices.iter().map(|c| amounts(c)).collect();
    wanted.sort();
    assert_eq!(gain_options(&e), wanted, "{name}");
    decide_gain(&mut e, P0, choices[pick]);
    assert!(e.pending().is_none());
    assert_pool(&e, P0, choices[pick]);
}

/// Player 0's Fiery Bow makes every rival lose 1 Life; the rest of the table is
/// given. Returns the game right after the power.
fn bow_vs(table: Table) -> Game {
    let mut e = table.card(P0, "Elfenbogen").start();
    let bow = card(&e, P0, "Elfenbogen");
    use_power(&mut e, P0, bow, 0);
    e
}

/// Player `p` uses the React power of one of their cards.
#[track_caller]
fn react_with_player(e: &mut Game, p: PlayerId, id: EntityId) {
    decide_where(
        e,
        p,
        |o| matches!(o, Effect::Use { source, .. } if *source == id),
    );
}

/// Player 1 uses the React power of one of its cards.
#[track_caller]
fn react_with(e: &mut Game, id: EntityId) {
    react_with_player(e, P1, id);
}

/// A React power "turn this card: ignore Life loss": it works while the card is
/// straight, turns it, and cannot be used while it is turned.
#[track_caller]
fn assert_turns_to_ignore_life_loss(name: &str) {
    let table = || Table::actions().pool(P1, &[(L, 3)]);
    let mut e = bow_vs(table().card(P1, name));
    let me = card(&e, P1, name);
    assert_eq!(
        options(&e).len(),
        2,
        "{name}: lose the Life or use the React"
    );
    assert!(matches!(
        options(&e)[0],
        Effect::Lose {
            res: LIFE,
            n: 1,
            ..
        }
    ));
    react_with(&mut e, me);
    assert!(e.pending().is_none());
    assert_pool(&e, P1, &[(L, 3)]);
    assert!(is_turned(&e, me), "{name} was turned");
    // Declining loses the Life.
    let mut e = bow_vs(table().card(P1, name));
    decide(&mut e, P1, 0);
    assert_pool(&e, P1, &[(L, 2)]);
    // A turned card cannot pay the turn: the Life is simply lost.
    let e = bow_vs(table().turned(P1, name));
    assert!(e.pending().is_none(), "{name} cannot react while turned");
    assert_pool(&e, P1, &[(L, 2)]);
}

/// Reorders the top cards of one of the decks after a "look at 3" power.
#[track_caller]
fn peek(e: &mut Game, deck: ZoneKind, perm: &[usize]) {
    decide_where(
        e,
        P0,
        |o| matches!(o, Effect::Reorder { zone, .. } if *zone == deck),
    );
    decide_where(
        e,
        P0,
        |o| matches!(o, Effect::Permute { perm: q, .. } if q == perm),
    );
}

// ---------------------------------------------------------------------------------------------------
// Mages
// ---------------------------------------------------------------------------------------------------

#[test]
fn healer_collects_calm_or_life_and_turns_to_ignore_life_loss() {
    assert_collect_choice("Heiler", &[&[(C, 1)], &[(L, 1)]], 0);
    assert_collect_choice("Heiler", &[&[(C, 1)], &[(L, 1)]], 1);
    assert_turns_to_ignore_life_loss("Heiler");
}

#[test]
fn transmuter_turns_in_two_essences_for_three_of_any_kind_but_gold() {
    let table = || Table::actions().card(P0, "Verwandlerin");
    let mut e = table().pool(P0, &[(E, 1), (L, 2)]).start();
    let mage = card(&e, P0, "Verwandlerin");
    assert_eq!(
        power_payments(&e, P0, mage, 0).len(),
        2,
        "two Life, or one Elan and one Life"
    );
    use_power_paying(&mut e, P0, mage, 0, &[(L, 2)]);
    assert!(is_turned(&e, mage));
    let choices = gain_options(&e);
    assert_eq!(choices.len(), 20, "every mix of 3 out of 4 kinds");
    assert!(choices.iter().all(|c| !c.contains_key(&G) && c.values().sum::<u32>() == 3));
    // Some of them can be the same kind as the ones turned in.
    decide_gain(&mut e, P0, &[(L, 3)]);
    assert_pool(&e, P0, &[(E, 1), (L, 3)]);
    // With fewer than two essences the power is not available.
    let e = table().pool(P0, &[(G, 1)]).start();
    assert!(!can_use(&e, P0, card(&e, P0, "Verwandlerin"), 0));
}

#[test]
fn scholar_turns_and_pays_one_essence_to_draw_a_card() {
    let mut e = Table::actions()
        .card(P0, "Gelehrter")
        .pool(P0, &[(E, 1), (G, 1)])
        .deck(P0, &["Nachtigall", "Wachhund"])
        .start();
    let mage = card(&e, P0, "Gelehrter");
    use_power_paying(&mut e, P0, mage, 0, &[(G, 1)]);
    assert_pool(&e, P0, &[(E, 1)]);
    assert_eq!(names(&e, P0, HAND), ["Nachtigall"]);
    assert_eq!(top_names(&e, &Owner::Player(P0), DECK, 5), ["Wachhund"]);
    assert!(is_turned(&e, mage));
}

#[test]
fn duelist_collects_elan_and_turns_with_death_to_put_gold_on_itself() {
    assert_collects("Duellant", &[(E, 1)]);
    let mut e = Table::actions().card(P0, "Duellant").pool(P0, &[(D, 1), (L, 1)]).start();
    let mage = card(&e, P0, "Duellant");
    use_power(&mut e, P0, mage, 0);
    assert_pool(&e, P0, &[(L, 1)]);
    assert_eq!(
        on_card(&e, mage, G),
        1,
        "the Gold is on the Duelist, not in the pool"
    );
    assert!(is_turned(&e, mage));
    // Death is needed.
    let e = Table::actions().card(P0, "Duellant").pool(P0, &[(L, 1)]).start();
    assert!(!can_use(&e, P0, card(&e, P0, "Duellant"), 0));
}

#[test]
fn druid_collects_life_and_straightens_only_creatures() {
    assert_collects("Druidin", &[(L, 1)]);
    let table = || {
        Table::actions()
            .card(P0, "Druidin")
            .turned(P0, "Wachhund")
            .turned(P0, "Zwergen-Spitzhacke")
    };
    let mut e = table().start();
    let (druid, dog, pickaxe) = (
        card(&e, P0, "Druidin"),
        card(&e, P0, "Wachhund"),
        card(&e, P0, "Zwergen-Spitzhacke"),
    );
    use_power(&mut e, P0, druid, 0);
    assert!(e.pending().is_none(), "the dog is the only turned creature");
    assert!(!is_turned(&e, dog));
    assert!(
        is_turned(&e, pickaxe),
        "an artifact that is not a creature stays turned"
    );
    assert!(is_turned(&e, druid));
    // With two turned creatures the player picks one.
    let mut e = table().turned(P0, "Nachtigall").start();
    let druid = card(&e, P0, "Druidin");
    use_power(&mut e, P0, druid, 0);
    assert_eq!(options(&e).len(), 2);
    let nightingale = card(&e, P0, "Nachtigall");
    decide_where(
        &mut e,
        P0,
        |o| matches!(o, Effect::WithCard { card, .. } if *card == nightingale),
    );
    assert!(!is_turned(&e, nightingale));
    assert!(is_turned(&e, card(&e, P0, "Wachhund")));
}

#[test]
fn artificer_takes_one_non_gold_essence_off_every_artifact() {
    let table = || Table::actions().card(P0, "Feuerwerker");
    // The Sacrificial Dagger costs Death and Gold: the Death is reduced.
    let mut e = table().hand(P0, &["Opferdolch"]).pool(P0, &[(G, 1)]).start();
    assert!(can_place(&e, P0, "Opferdolch"));
    place(&mut e, P0, "Opferdolch", &[(G, 1)]);
    assert!(has(&e, P0, TABLEAU, "Opferdolch"));
    assert_pool(&e, P0, &[]);
    // The Gold part cannot be reduced.
    let e = table().hand(P0, &["Opferdolch"]).pool(P0, &[(D, 1)]).start();
    assert!(!can_place(&e, P0, "Opferdolch"));
    // Any-essence parts are reduced too (Tree of Life: 2 any and Life).
    let mut e = table().hand(P0, &["Baum des Lebens"]).pool(P0, &[(L, 1), (E, 1)]).start();
    place(&mut e, P0, "Baum des Lebens", &[(L, 1), (E, 1)]);
    assert_pool(&e, P0, &[]);
    // Monuments and places of power are not artifacts: their price stands.
    let e = table().row(&["Orakel", "Obelisk"]).pool(P0, &[(G, 3)]).start();
    assert!(!can_claim(&e, P0, "Orakel"));
    let e = table().pool(P0, &[(D, 8)]).start();
    assert!(!can_claim(&e, P0, "Katakomben der Toten"));
}

#[test]
fn seer_collects_calm_and_looks_at_three_cards_of_either_deck() {
    assert_collects("Seherin", &[(C, 1)]);
    let table = || {
        Table::actions().card(P0, "Seherin").deck(
            P0,
            &["Nachtigall", "Wachhund", "Magische Scherbe", "Prisma"],
        )
    };
    let mut e = table().start();
    let seer = card(&e, P0, "Seherin");
    use_power(&mut e, P0, seer, 0);
    assert_eq!(
        options(&e).len(),
        2,
        "the artifact deck or the monument deck"
    );
    peek(&mut e, DECK, &[2, 1, 0]);
    assert_eq!(
        top_names(&e, &Owner::Player(P0), DECK, 4),
        ["Magische Scherbe", "Wachhund", "Nachtigall", "Prisma"],
        "the top three were put back reversed"
    );
    assert!(is_turned(&e, seer));
    // The monument deck works the same way.
    let mut e = table().start();
    let before = top_names(&e, &Owner::Shared, MONUMENT_DECK, 3);
    let seer = card(&e, P0, "Seherin");
    use_power(&mut e, P0, seer, 0);
    peek(&mut e, MONUMENT_DECK, &[1, 0, 2]);
    assert_eq!(
        top_names(&e, &Owner::Shared, MONUMENT_DECK, 3),
        [before[1].clone(), before[0].clone(), before[2].clone()]
    );
}

#[test]
fn seer_shuffles_the_discard_pile_under_a_short_deck_before_looking() {
    // One card is left in the deck; the discard pile holds three.
    let mut e = Table::actions()
        .card(P0, "Seherin")
        .deck(P0, &["Nachtigall"])
        .discard(P0, &["Wachhund", "Magische Scherbe", "Prisma"])
        .start();
    let seer = card(&e, P0, "Seherin");
    use_power(&mut e, P0, seer, 0);
    peek(&mut e, DECK, &[0, 1, 2]);
    assert!(names(&e, P0, DISCARD).is_empty(), "the discard pile went into the deck");
    let deck = top_names(&e, &Owner::Player(P0), DECK, 4);
    assert_eq!(deck.len(), 4);
    assert_eq!(deck[0], "Nachtigall", "the card that was left stays on top");
}

#[test]
fn necromancer_collects_death_and_turns_with_two_life_for_three_death_on_itself() {
    assert_collects("Nekromantin", &[(D, 1)]);
    let mut e = Table::actions().card(P0, "Nekromantin").pool(P0, &[(L, 3)]).start();
    let mage = card(&e, P0, "Nekromantin");
    use_power(&mut e, P0, mage, 0);
    assert_pool(&e, P0, &[(L, 1)]);
    assert_eq!(on_card(&e, mage, D), 3);
    assert!(is_turned(&e, mage));
    let e = Table::actions().card(P0, "Nekromantin").pool(P0, &[(L, 1), (D, 5)]).start();
    assert!(
        !can_use(&e, P0, card(&e, P0, "Nekromantin"), 0),
        "it needs Life, not Death"
    );
}

#[test]
fn witch_collects_life_or_death_and_straightens_any_component() {
    assert_collect_choice("Hexe", &[&[(L, 1)], &[(D, 1)]], 0);
    assert_collect_choice("Hexe", &[&[(L, 1)], &[(D, 1)]], 1);
    let table = || Table::actions().card(P0, "Hexe").pool(P0, &[(E, 2), (C, 1)]);
    // The witch turns itself as a cost, so it can straighten itself.
    let mut e = table().start();
    let witch = card(&e, P0, "Hexe");
    use_power_paying(&mut e, P0, witch, 0, &[(E, 2)]);
    assert!(e.pending().is_none());
    assert!(!is_turned(&e, witch));
    assert_pool(&e, P0, &[(C, 1)]);
    // With another turned component the player picks.
    let mut e = table().turned(P0, "Nachtigall").start();
    let (witch, bird) = (card(&e, P0, "Hexe"), card(&e, P0, "Nachtigall"));
    use_power_paying(&mut e, P0, witch, 0, &[(E, 2)]);
    assert_eq!(options(&e).len(), 2);
    decide_where(
        &mut e,
        P0,
        |o| matches!(o, Effect::WithCard { card, .. } if *card == bird),
    );
    assert!(!is_turned(&e, bird));
    assert!(is_turned(&e, witch));
}

#[test]
fn alchemist_has_two_powers_that_both_turn_it() {
    let table = || Table::actions().card(P0, "Alchemist");
    // Four essences of any kind become two Gold.
    let mut e = table().pool(P0, &[(E, 1), (L, 1), (C, 1), (D, 1)]).start();
    let mage = card(&e, P0, "Alchemist");
    assert!(can_use(&e, P0, mage, 0));
    use_power(&mut e, P0, mage, 0);
    assert_pool(&e, P0, &[(G, 2)]);
    assert!(is_turned(&e, mage));
    assert!(!can_use(&e, P0, mage, 1), "both powers turn the Alchemist");
    // For free it makes one essence of any kind but Gold.
    let mut e = table().start();
    let mage = card(&e, P0, "Alchemist");
    use_power(&mut e, P0, mage, 1);
    let choices = gain_options(&e);
    let mut wanted = [E, L, C, D].map(|r| amounts(&[(r, 1)]));
    wanted.sort();
    assert_eq!(choices, wanted);
    decide_gain(&mut e, P0, &[(C, 1)]);
    assert_pool(&e, P0, &[(C, 1)]);
    assert!(!can_use(&e, P0, mage, 0));
}

// ---------------------------------------------------------------------------------------------------
// Monuments
// ---------------------------------------------------------------------------------------------------

const MONUMENTS: [&str; 10] = [
    "Orakel",
    "Obelisk",
    "Salomos Schatzkammer",
    "Mausoleum",
    "Hängende Gärten",
    "Bibliothek",
    "Große Pyramide",
    "Goldene Statue",
    "Tempel",
    "Koloss",
];

#[test]
fn every_monument_is_claimed_for_four_gold_and_the_row_is_refilled() {
    for name in MONUMENTS {
        let other = if name == "Orakel" {
            "Obelisk"
        } else {
            "Orakel"
        };
        let table = || Table::actions().row(&[name, other]);
        // Three Gold are not enough, four are.
        let e = table().pool(P0, &[(G, 3)]).start();
        assert!(!can_claim(&e, P0, name), "{name} costs 4 Gold");
        let mut e = table().pool(P0, &[(G, 4), (E, 1)]).start();
        claim(&mut e, P0, name, &[(G, 4)]);
        assert!(has(&e, P0, TABLEAU, name), "{name} is in the tableau");
        assert!(
            !is_turned(&e, card(&e, P0, name)),
            "components begin play straightened"
        );
        assert_eq!(
            zone_cards(&e, &Owner::Shared, MONUMENT_ROW).len(),
            2,
            "{name}: refilled"
        );
        assert_eq!(pool(&e, P0, G), 0);
        settle(&mut e);
        assert_eq!(
            e.current_player(),
            Some(P1),
            "claiming is the turn's action"
        );
    }
}

#[test]
fn a_monument_can_be_claimed_blind_from_the_top_of_the_deck() {
    let mut e = Table::actions().row(&["Orakel", "Obelisk"]).pool(P0, &[(G, 4)]).start();
    let top = *zone_cards(&e, &Owner::Shared, MONUMENT_DECK).last().unwrap();
    let name = def_name(&e, top);
    claim(&mut e, P0, &name, &[(G, 4)]);
    settle(&mut e);
    assert!(has(&e, P0, TABLEAU, &name));
    assert_eq!(
        zone_cards(&e, &Owner::Shared, MONUMENT_ROW).len(),
        2,
        "the row is unchanged"
    );
}

#[test]
fn oracle_is_worth_two_and_looks_at_three_cards_of_either_deck() {
    let table = || {
        Table::actions()
            .card(P0, "Orakel")
            .deck(P0, &["Nachtigall", "Wachhund", "Magische Scherbe"])
    };
    let mut e = table().start();
    assert_eq!(score(&e, P0), 2);
    let oracle = card(&e, P0, "Orakel");
    use_power(&mut e, P0, oracle, 0);
    peek(&mut e, DECK, &[1, 2, 0]);
    assert_eq!(
        top_names(&e, &Owner::Player(P0), DECK, 3),
        ["Wachhund", "Magische Scherbe", "Nachtigall"]
    );
    assert!(is_turned(&e, oracle));
    let mut e = table().start();
    let before = top_names(&e, &Owner::Shared, MONUMENT_DECK, 3);
    let oracle = card(&e, P0, "Orakel");
    use_power(&mut e, P0, oracle, 0);
    peek(&mut e, MONUMENT_DECK, &[2, 1, 0]);
    let after = top_names(&e, &Owner::Shared, MONUMENT_DECK, 3);
    assert_eq!(
        after,
        [before[2].clone(), before[1].clone(), before[0].clone()]
    );
}

#[test]
fn obelisk_gives_six_essences_other_than_gold_when_it_is_claimed() {
    let mut e = Table::actions().row(&["Obelisk", "Orakel"]).pool(P0, &[(G, 4)]).start();
    claim(&mut e, P0, "Obelisk", &[(G, 4)]);
    assert_eq!(options(&e).len(), 84, "every mix of 6 out of 4 kinds");
    assert!(gain_options(&e).iter().all(|c| !c.contains_key(&G)));
    decide_gain(&mut e, P0, &[(E, 2), (L, 2), (C, 2)]);
    assert_pool(&e, P0, &[(E, 2), (L, 2), (C, 2)]);
    assert_eq!(score(&e, P0), 1);
    assert_eq!(e.current_player(), Some(P1));
}

#[test]
fn solomons_treasury_turns_for_one_gold() {
    let mut e = Table::actions().card(P0, "Salomos Schatzkammer").start();
    let treasury = card(&e, P0, "Salomos Schatzkammer");
    assert_eq!(score(&e, P0), 1);
    use_power(&mut e, P0, treasury, 0);
    assert_pool(&e, P0, &[(G, 1)]);
    assert!(is_turned(&e, treasury));
    assert!(!can_use(&e, P0, treasury, 0));
}

#[test]
fn mausoleum_pays_any_essence_to_put_death_on_itself_and_can_be_used_again() {
    let mut e = Table::actions()
        .card(P0, "Mausoleum")
        .pool(P0, &[(E, 2)])
        .hand(P0, &["Prisma"])
        .hand(P1, &["Nachtigall"])
        .start();
    let tomb = card(&e, P0, "Mausoleum");
    assert_eq!(score(&e, P0), 2);
    use_power(&mut e, P0, tomb, 0);
    assert_eq!(on_card(&e, tomb, D), 1);
    assert_pool(&e, P0, &[(E, 1)]);
    assert!(
        !is_turned(&e, tomb),
        "the power does not turn the Mausoleum"
    );
    rival_acts(&mut e);
    use_power(&mut e, P0, tomb, 0);
    assert_eq!(on_card(&e, tomb, D), 2);
    assert_pool(&e, P0, &[]);
    assert!(!can_use(&e, P0, tomb, 0), "nothing left to pay");
}

#[test]
fn hanging_gardens_collect_three_essences_other_than_gold() {
    let mut e = Table::round().card(P0, "Hängende Gärten").start();
    assert_eq!(score(&e, P0), 1);
    assert_eq!(options(&e).len(), 20);
    assert!(gain_options(&e).iter().all(|c| !c.contains_key(&G)));
    decide_gain(&mut e, P0, &[(L, 1), (C, 1), (D, 1)]);
    assert_pool(&e, P0, &[(L, 1), (C, 1), (D, 1)]);
}

#[test]
fn library_turns_to_draw_a_card() {
    let mut e = Table::actions().card(P0, "Bibliothek").deck(P0, &["Wachhund"]).start();
    let library = card(&e, P0, "Bibliothek");
    assert_eq!(score(&e, P0), 1);
    use_power(&mut e, P0, library, 0);
    assert_eq!(names(&e, P0, HAND), ["Wachhund"]);
    assert!(is_turned(&e, library));
}

#[test]
fn great_pyramid_is_worth_three_points_and_does_nothing_else() {
    let e = Table::actions().card(P0, "Große Pyramide").start();
    assert_eq!(score(&e, P0), 3);
    let pyramid = card(&e, P0, "Große Pyramide");
    assert_eq!(
        abilities_of(&e, pyramid),
        0,
        "it has no powers, collect or reactions"
    );
}

#[test]
fn temple_collects_life_and_turns_to_ignore_life_loss() {
    assert_collects("Tempel", &[(L, 1)]);
    assert_turns_to_ignore_life_loss("Tempel");
    assert_eq!(score(&Table::actions().card(P0, "Tempel").start(), P0), 2);
}

#[test]
fn colossus_turns_with_any_essence_to_put_gold_on_itself() {
    let mut e = Table::actions().card(P0, "Koloss").pool(P0, &[(D, 1), (C, 1)]).start();
    let colossus = card(&e, P0, "Koloss");
    assert_eq!(score(&e, P0), 2);
    use_power_paying(&mut e, P0, colossus, 0, &[(D, 1)]);
    assert_eq!(on_card(&e, colossus, G), 1);
    assert_pool(&e, P0, &[(C, 1)]);
    assert!(is_turned(&e, colossus));
}

// ---------------------------------------------------------------------------------------------------
// Magic items
// ---------------------------------------------------------------------------------------------------

#[test]
fn research_turns_and_pays_one_essence_to_draw_a_card() {
    let mut e = Table::actions()
        .card(P0, "Forschung")
        .pool(P0, &[(L, 1)])
        .deck(P0, &["Prisma", "Wachhund"])
        .start();
    let item = card(&e, P0, "Forschung");
    use_power(&mut e, P0, item, 0);
    assert_eq!(names(&e, P0, HAND), ["Prisma"]);
    assert_pool(&e, P0, &[]);
    assert!(is_turned(&e, item));
}

#[test]
fn transmutation_turns_three_essences_for_three_others_but_gold() {
    let mut e = Table::actions().card(P0, "Umwandlung").pool(P0, &[(E, 2), (G, 1)]).start();
    let item = card(&e, P0, "Umwandlung");
    assert_eq!(
        power_payments(&e, P0, item, 0).len(),
        1,
        "only 3 essences are held"
    );
    use_power(&mut e, P0, item, 0);
    assert_eq!(options(&e).len(), 20);
    assert!(gain_options(&e).iter().all(|c| !c.contains_key(&G)));
    decide_gain(&mut e, P0, &[(D, 3)]);
    assert_pool(&e, P0, &[(D, 3)]);
    assert!(is_turned(&e, item));
    let e = Table::actions().card(P0, "Umwandlung").pool(P0, &[(E, 2)]).start();
    assert!(
        !can_use(&e, P0, card(&e, P0, "Umwandlung"), 0),
        "2 essences are not enough"
    );
}

#[test]
fn death_life_collects_either_one() {
    assert_collect_choice("Tod / Leben", &[&[(D, 1)], &[(L, 1)]], 0);
    assert_collect_choice("Tod / Leben", &[&[(D, 1)], &[(L, 1)]], 1);
}

#[test]
fn calm_elan_collects_either_one() {
    assert_collect_choice("Ruhe / Elan", &[&[(C, 1)], &[(E, 1)]], 0);
    assert_collect_choice("Ruhe / Elan", &[&[(C, 1)], &[(E, 1)]], 1);
}

/// Picks the hand card to discard in a "discard n cards" decision.
#[track_caller]
fn discard_pick(e: &mut Game, p: PlayerId, id: EntityId) {
    decide_where(e, p, |o| match o {
        Effect::Sequence(steps) => {
            matches!(steps.first(), Some(Effect::WithCard { card, .. }) if *card == id)
        },
        _ => false,
    });
}

#[test]
fn divination_draws_three_cards_then_discards_three() {
    let mut e = Table::actions()
        .card(P0, "Wahrsagen")
        .hand(P0, &["Prisma", "Wachhund"])
        .deck(
            P0,
            &[
                "Nachtigall",
                "Hand des Ruhmes",
                "Füllhorn",
                "Magische Scherbe",
            ],
        )
        .start();
    let item = card(&e, P0, "Wahrsagen");
    use_power(&mut e, P0, item, 0);
    assert_eq!(names(&e, P0, HAND).len(), 5, "three cards were drawn");
    for name in ["Prisma", "Wachhund", "Nachtigall"] {
        let id = find(&e, P0, HAND, name);
        discard_pick(&mut e, P0, id);
    }
    assert!(e.pending().is_none());
    assert_eq!(names(&e, P0, HAND), ["Füllhorn", "Hand des Ruhmes"]);
    assert_eq!(names(&e, P0, DISCARD), ["Nachtigall", "Prisma", "Wachhund"]);
    assert_eq!(
        top_names(&e, &Owner::Player(P0), DECK, 5),
        ["Magische Scherbe"]
    );
    assert!(is_turned(&e, item));
    // Discarding as a power's effect gives no Gold: the pool stays empty.
    assert_pool(&e, P0, &[]);
}

#[test]
fn divination_with_a_short_deck_discards_only_as_many_as_it_drew() {
    // The deck and the discard pile together hold one card: one is drawn, so
    // one is discarded, and the hand ends with the two cards it started with.
    let mut e = Table::actions()
        .card(P0, "Wahrsagen")
        .hand(P0, &["Prisma", "Wachhund"])
        .deck(P0, &["Nachtigall"])
        .start();
    let item = card(&e, P0, "Wahrsagen");
    use_power(&mut e, P0, item, 0);
    settle(&mut e);
    assert_eq!(names(&e, P0, HAND).len(), 2);
    assert_eq!(names(&e, P0, DISCARD).len(), 1);
}

#[test]
fn alchemy_turns_four_essences_for_two_gold() {
    let mut e = Table::actions().card(P0, "Alchemie").pool(P0, &[(E, 2), (D, 2), (L, 1)]).start();
    let item = card(&e, P0, "Alchemie");
    use_power_paying(&mut e, P0, item, 0, &[(E, 2), (D, 2)]);
    assert_pool(&e, P0, &[(L, 1), (G, 2)]);
    assert!(is_turned(&e, item));
    let e = Table::actions().card(P0, "Alchemie").pool(P0, &[(E, 3)]).start();
    assert!(!can_use(&e, P0, card(&e, P0, "Alchemie"), 0));
}

#[test]
fn protection_turns_to_ignore_life_loss() {
    assert_turns_to_ignore_life_loss("Schutz");
}

#[test]
fn reanimate_turns_with_an_essence_to_straighten_any_component() {
    let mut e = Table::actions()
        .card(P0, "Wiederbelebung")
        .turned(P0, "Nachtigall")
        .pool(P0, &[(C, 1)])
        .start();
    let (item, bird) = (card(&e, P0, "Wiederbelebung"), card(&e, P0, "Nachtigall"));
    use_power(&mut e, P0, item, 0);
    // The item itself is turned now too and could be picked.
    assert_eq!(options(&e).len(), 2);
    decide_where(
        &mut e,
        P0,
        |o| matches!(o, Effect::WithCard { card, .. } if *card == bird),
    );
    assert!(!is_turned(&e, bird));
    assert!(is_turned(&e, item));
    assert_pool(&e, P0, &[]);
}

// ---------------------------------------------------------------------------------------------------
// Places of power
// ---------------------------------------------------------------------------------------------------

const PLACE_COSTS: [(&str, &[Qty]); 5] = [
    ("Verfluchte Schmiede", &[(E, 6), (D, 3)]),
    ("Heiliger Hain", &[(L, 8), (C, 4)]),
    ("Drachenhort", &[(E, 3), (L, 3), (C, 3), (D, 3)]),
    ("Korallenschloss", &[(E, 5), (L, 5), (C, 5)]),
    ("Katakomben der Toten", &[(D, 9)]),
];

#[test]
fn every_place_of_power_is_claimed_for_its_cost_and_not_replaced() {
    for (name, cost) in PLACE_COSTS {
        let mut e = Table::actions().pool(P0, cost).start();
        assert_eq!(zone_cards(&e, &Owner::Shared, PLACES).len(), 5);
        claim(&mut e, P0, name, cost);
        assert!(has(&e, P0, TABLEAU, name), "{name}");
        assert_pool(&e, P0, &[]);
        assert_eq!(
            zone_cards(&e, &Owner::Shared, PLACES).len(),
            4,
            "{name} is not replaced"
        );
        // One essence short of any part of the price.
        for at in 0..cost.len() {
            let mut short = cost.to_vec();
            short[at].1 -= 1;
            let e = Table::actions().pool(P0, &short).start();
            assert!(!can_claim(&e, P0, name), "{name} needs the full price");
        }
    }
}

#[test]
fn cursed_forge_collects_by_paying_death_or_turning_and_scores_for_its_gold() {
    let table = || Table::round().card(P0, "Verfluchte Schmiede");
    // Pay the curse: 1 Death goes to the supply, the Forge stays straight.
    let mut e = table().pool(P0, &[(D, 1)]).start();
    let forge = card(&e, P0, "Verfluchte Schmiede");
    assert_eq!(options(&e).len(), 2);
    decide_where(&mut e, P0, |o| {
        matches!(o, Effect::Pay { res: DEATH, n: 1 })
    });
    assert_pool(&e, P0, &[]);
    assert!(!is_turned(&e, forge));
    // Or turn it and keep the Death.
    let mut e = table().pool(P0, &[(D, 1)]).start();
    let forge = card(&e, P0, "Verfluchte Schmiede");
    decide_where(&mut e, P0, |o| matches!(o, Effect::Exhaust(_)));
    assert_pool(&e, P0, &[(D, 1)]);
    assert!(is_turned(&e, forge));
    // Without Death turning is the only way.
    let mut e = table().start();
    let forge = card(&e, P0, "Verfluchte Schmiede");
    assert_eq!(options(&e).len(), 1, "only turning is offered");
    decide(&mut e, P0, 0);
    assert!(is_turned(&e, forge));
    // 2 Elan and 1 Gold put 1 Gold on the Forge: 1 point for the Forge and 1
    // for each Gold on it.
    let mut e = Table::actions()
        .loaded(P0, "Verfluchte Schmiede", &[(G, 2)])
        .pool(P0, &[(E, 2), (G, 1)])
        .start();
    let forge = card(&e, P0, "Verfluchte Schmiede");
    assert_eq!(score(&e, P0), 3);
    use_power(&mut e, P0, forge, 0);
    assert_pool(&e, P0, &[]);
    assert_eq!(on_card(&e, forge, G), 3);
    assert!(!is_turned(&e, forge), "this power does not turn the Forge");
    assert_eq!(score(&e, P0), 4);
}

#[test]
fn sacred_grove_gives_five_life_or_puts_life_on_itself_by_turning_a_creature() {
    let table = || Table::actions().card(P0, "Heiliger Hain");
    let mut e = table().pool(P0, &[(C, 1)]).start();
    let grove = card(&e, P0, "Heiliger Hain");
    assert_eq!(score(&e, P0), 2);
    use_power(&mut e, P0, grove, 0);
    assert_pool(&e, P0, &[(L, 5)]);
    assert!(is_turned(&e, grove));
    // The second power needs a straight creature, and turns it.
    let e = table().card(P0, "Zwergen-Spitzhacke").start();
    assert!(
        !can_use(&e, P0, card(&e, P0, "Heiliger Hain"), 1),
        "no creature to turn"
    );
    let mut e = table().card(P0, "Nachtigall").card(P0, "Wachhund").start();
    let (grove, dog) = (card(&e, P0, "Heiliger Hain"), card(&e, P0, "Wachhund"));
    assert_eq!(
        power_payments(&e, P0, grove, 1).len(),
        2,
        "either creature can be turned"
    );
    use_power_where(&mut e, P0, grove, 1, |p| p.turned == [dog]);
    assert_eq!(on_card(&e, grove, L), 1);
    assert!(is_turned(&e, grove) && is_turned(&e, dog));
    assert!(!is_turned(&e, card(&e, P0, "Nachtigall")));
    assert_pool(&e, P0, &[]);
    assert_eq!(
        score(&e, P0),
        4,
        "1 more point for the Life on it, plus the 1 of the Nightingale"
    );
}

#[test]
fn dragons_lair_discounts_dragons_makes_gold_and_scores_for_it() {
    let table = || Table::actions().card(P0, "Drachenhort");
    let e = Table::actions().loaded(P0, "Drachenhort", &[(G, 4)]).start();
    assert_eq!(
        score(&e, P0),
        4,
        "no points of its own, 1 for each Gold on it"
    );
    // 2 Gold into the pool, turning the Lair.
    let mut e = table().start();
    let lair = card(&e, P0, "Drachenhort");
    use_power(&mut e, P0, lair, 0);
    assert_pool(&e, P0, &[(G, 2)]);
    assert!(is_turned(&e, lair));
    assert!(!can_use(&e, P0, lair, 1));
    // 2 Gold on the Lair by turning a Dragon too.
    let mut e = table().card(P0, "Wasserdrache").start();
    let (lair, dragon) = (card(&e, P0, "Drachenhort"), card(&e, P0, "Wasserdrache"));
    use_power(&mut e, P0, lair, 1);
    assert_eq!(on_card(&e, lair, G), 2);
    assert_pool(&e, P0, &[]);
    assert!(is_turned(&e, lair) && is_turned(&e, dragon));
    assert_eq!(
        score(&e, P0),
        1 + 2,
        "2 for the Gold on the Lair and 1 for the Water Dragon"
    );
    // A Dragon is needed for the second power.
    let e = table().card(P0, "Nachtigall").start();
    assert!(!can_use(&e, P0, card(&e, P0, "Drachenhort"), 1));
}

#[test]
fn dragons_lair_takes_three_essences_off_dragons_only() {
    let table = || Table::actions().card(P0, "Drachenhort");
    let mut e = table().hand(P0, &["Wasserdrache"]).pool(P0, &[(C, 3)]).start();
    place(&mut e, P0, "Wasserdrache", &[(C, 3)]);
    assert!(has(&e, P0, TABLEAU, "Wasserdrache"));
    let e = table().hand(P0, &["Wasserdrache"]).pool(P0, &[(C, 2)]).start();
    assert!(
        !can_place(&e, P0, "Wasserdrache"),
        "6 Calm minus 3 is still 3"
    );
    // Other artifacts keep their price.
    let e = table().hand(P0, &["Winddrache", "Hand des Ruhmes"]).pool(P0, &[(D, 1)]).start();
    assert!(
        !can_place(&e, P0, "Hand des Ruhmes"),
        "Life and Death are both needed"
    );
}

#[test]
fn discounts_add_up_but_a_price_never_drops_below_zero() {
    // The Lair and the Bridle take 6 off the 6 Calm of the Water Dragon.
    let mut e = Table::actions()
        .card(P0, "Drachenhort")
        .card(P0, "Drachenzaum")
        .hand(P0, &["Wasserdrache"])
        .start();
    place(&mut e, P0, "Wasserdrache", &[]);
    assert!(has(&e, P0, TABLEAU, "Wasserdrache"));
    // With the Artificer as well (7 off a price of 6) it still costs nothing.
    let mut e = Table::actions()
        .card(P0, "Drachenhort")
        .card(P0, "Drachenzaum")
        .card(P0, "Feuerwerker")
        .hand(P0, &["Feuerdrache"])
        .pool(P0, &[(G, 1)])
        .start();
    place(&mut e, P0, "Feuerdrache", &[]);
    assert_pool(&e, P0, &[(G, 1)]);
    // The Lair and the Artificer: 4 off the 6 Calm of the Water Dragon.
    let mut e = Table::actions()
        .card(P0, "Drachenhort")
        .card(P0, "Feuerwerker")
        .hand(P0, &["Wasserdrache"])
        .pool(P0, &[(C, 2)])
        .start();
    place(&mut e, P0, "Wasserdrache", &[(C, 2)]);
}

#[test]
fn dragon_bridle_discounts_dragons_and_ignores_a_dragons_life_loss() {
    let mut e = Table::actions()
        .card(P0, "Drachenzaum")
        .hand(P0, &["Wasserdrache", "Hand des Ruhmes"])
        .pool(P0, &[(C, 3)])
        .start();
    assert_eq!(score(&e, P0), 1);
    place(&mut e, P0, "Wasserdrache", &[(C, 3)]);
    // The Wind Dragon attacks; the Bridle (turn) ignores the loss. Without a
    // hand the Wind Dragon's own escape is not available.
    let mut e = Table::actions()
        .card(P0, "Winddrache")
        .card(P1, "Drachenzaum")
        .pool(P1, &[(L, 3)])
        .start();
    let dragon = card(&e, P0, "Winddrache");
    use_power(&mut e, P0, dragon, 0);
    let bridle = card(&e, P1, "Drachenzaum");
    assert_eq!(options(&e).len(), 2);
    react_with(&mut e, bridle);
    assert_pool(&e, P1, &[(L, 3)]);
    assert!(is_turned(&e, bridle));
    assert!(e.pending().is_none());
}

#[test]
fn coral_castle_is_worth_three_ignores_life_loss_and_checks_victory() {
    assert_turns_to_ignore_life_loss("Korallenschloss");
    // 3 + 3 + 2 + 2 = 10 points: the check ends the game at once.
    let table = |pyramid_and_more: &[&str]| {
        let mut t = Table::actions().card(P0, "Korallenschloss");
        for name in pyramid_and_more {
            t = t.card(P0, name);
        }
        t
    };
    let mut e = table(&["Große Pyramide", "Orakel", "Tempel"]).start();
    assert_eq!(score(&e, P0), 10);
    assert_eq!(e.winners(), None);
    let castle = card(&e, P0, "Korallenschloss");
    use_power(&mut e, P0, castle, 0);
    assert_eq!(e.winners(), Some(&[P0][..]));
    // 9 points are not enough.
    let mut e = table(&["Große Pyramide", "Orakel", "Salomos Schatzkammer"]).start();
    assert_eq!(score(&e, P0), 9);
    let castle = card(&e, P0, "Korallenschloss");
    use_power(&mut e, P0, castle, 0);
    assert_eq!(e.winners(), None);
    assert!(is_turned(&e, castle));
}

#[test]
fn catacombs_collect_death_put_death_on_themselves_and_score_for_it() {
    assert_collects("Katakomben der Toten", &[(D, 1)]);
    let table = || Table::actions().loaded(P0, "Katakomben der Toten", &[(D, 2)]);
    let mut e = table().pool(P0, &[(D, 7)]).start();
    let pit = card(&e, P0, "Katakomben der Toten");
    assert_eq!(score(&e, P0), 2, "1 point for each Death on it");
    // 5 Death pay for 1 Death on it, and the power does not turn it.
    use_power(&mut e, P0, pit, 0);
    assert_pool(&e, P0, &[(D, 2)]);
    assert_eq!(on_card(&e, pit, D), 3);
    assert!(!is_turned(&e, pit));
    assert_eq!(score(&e, P0), 3);
    // Turning is free and puts 1 Death on it too.
    let mut e = table().start();
    let pit = card(&e, P0, "Katakomben der Toten");
    assert!(!can_use(&e, P0, pit, 0), "5 Death are needed");
    use_power(&mut e, P0, pit, 1);
    assert_eq!(on_card(&e, pit, D), 3);
    assert!(is_turned(&e, pit));
    assert_eq!(score(&e, P0), 3, "turned places still score");
}

#[test]
fn taking_the_essences_off_a_place_of_power_loses_its_points() {
    let mut e = Table::round().loaded(P0, "Katakomben der Toten", &[(D, 3)]).start();
    let pit = card(&e, P0, "Katakomben der Toten");
    assert_eq!(score(&e, P0), 3);
    assert!(options(&e).len() == 2, "take them all or none of them");
    take_from(&mut e, P0, pit);
    assert_eq!(on_card(&e, pit, D), 0);
    assert_pool(&e, P0, &[(D, 4)]);
    assert_eq!(score(&e, P0), 0);
}

// ---------------------------------------------------------------------------------------------------
// Victory checks, the Golden Statue and the first player token
// ---------------------------------------------------------------------------------------------------

/// Player 1 has 8 points (Pyramid, Oracle, Temple, Statue) and 3 Gold; both
/// players pass, so the victory check of the first round is pending.
fn statue_at_the_victory_check(p1_cards: &[&str], gold: u32) -> Game {
    let mut table = Table::round().pool(P1, &[(G, gold)]);
    for name in p1_cards {
        table = table.card(P1, name);
    }
    let mut e = table.start();
    pass(&mut e, P0);
    pass(&mut e, P1);
    e
}

fn use_statue(e: &mut Game) {
    decide_where(
        e,
        P1,
        |o| matches!(o, Effect::Sequence(steps) if matches!(steps.first(), Some(Effect::Use { .. }))),
    );
}

#[test]
fn the_last_to_pass_does_not_swap_the_item_when_the_game_is_already_won() {
    let won = ["Große Pyramide", "Korallenschloss", "Orakel", "Goldene Statue"];
    let table = |cards: &[&str]| {
        let mut table = Table::round();
        for name in cards {
            table = table.card(P1, name);
        }
        table.start()
    };
    let mut e = table(&won);
    pass(&mut e, P0);
    e.apply(P1, Action::Pass).expect("passing is possible");
    assert!(!picks_an_item(&e), "10 points are there: no item to swap");
    // With fewer points the swap still happens.
    let mut e = table(&won[..2]);
    pass(&mut e, P0);
    e.apply(P1, Action::Pass).expect("passing is possible");
    assert!(picks_an_item(&e));
}

#[test]
fn golden_statue_turns_with_three_gold_for_three_temporary_points() {
    let all = ["Große Pyramide", "Orakel", "Tempel", "Goldene Statue"];
    let mut e = statue_at_the_victory_check(&all, 3);
    assert_eq!(score(&e, P1), 8);
    assert_eq!(
        e.pending().unwrap().players,
        [P1],
        "only the Statue's owner may react"
    );
    assert_eq!(options(&e).len(), 2, "do nothing, or use the Statue");
    let statue = card(&e, P1, "Goldene Statue");
    use_statue(&mut e);
    assert!(e.winners() == Some(&[P1][..]), "8 + 3 points win the game");
    assert_eq!(score(&e, P1), 11);
    assert_eq!(pool(&e, P1, G), 0, "the 3 Gold were paid");
    assert!(is_turned(&e, statue));
    let scored = e.events().iter().any(
        |ev| matches!(ev, Event::CounterChanged { counter, delta: 3, .. } if *counter == TEMP_VP),
    );
    assert!(scored);
}

#[test]
fn without_the_statue_the_check_finds_no_winner_and_the_round_goes_on() {
    let all = ["Große Pyramide", "Orakel", "Tempel", "Goldene Statue"];
    let mut e = statue_at_the_victory_check(&all, 3);
    decide(&mut e, P1, 0);
    assert_eq!(e.winners(), None);
    assert_eq!(e.state().flow.round, 2);
    assert_eq!(pool(&e, P1, G), 3);
    assert_eq!(score(&e, P1), 8);
}

#[test]
fn the_statues_points_are_forgotten_after_the_check() {
    // 3 + 2 + 1 = 6 points, 9 with the Statue: not enough.
    let mut e = statue_at_the_victory_check(&["Große Pyramide", "Orakel", "Goldene Statue"], 3);
    use_statue(&mut e);
    assert_eq!(e.winners(), None);
    assert_eq!(temp_points(&e, P1), 0, "temporary points are gone");
    assert_eq!(score(&e, P1), 6);
    assert_eq!(pool(&e, P1, G), 0, "the Gold was spent for nothing");
    // The Statue is straightened again by the cleanup.
    assert!(!is_turned(&e, card(&e, P1, "Goldene Statue")));
}

#[test]
fn the_statue_needs_three_gold_and_to_be_straight() {
    let all = ["Große Pyramide", "Orakel", "Tempel", "Goldene Statue"];
    let e = statue_at_the_victory_check(&all, 2);
    assert!(e.pending().is_none(), "2 Gold are not enough");
    assert_eq!(e.state().flow.round, 2);
    let mut table = Table::round().pool(P1, &[(G, 3)]).turned(P1, "Goldene Statue");
    for name in &all[..3] {
        table = table.card(P1, name);
    }
    let mut e = table.start();
    pass(&mut e, P0);
    pass(&mut e, P1);
    assert!(e.pending().is_none(), "a turned Statue cannot pay its turn");
}

#[test]
fn the_statue_answers_a_victory_check_in_the_middle_of_the_round() {
    let table = |statue: bool| {
        let mut t = Table::actions().pool(P1, &[(G, 3)]);
        for name in [
            "Korallenschloss",
            "Koloss",
            "Mausoleum",
            "Bibliothek",
            "Obelisk",
        ] {
            t = t.card(P0, name);
        }
        t = t.card(P0, "Salomos Schatzkammer");
        for name in ["Große Pyramide", "Orakel", "Tempel"] {
            t = t.card(P1, name);
        }
        if statue {
            t.card(P1, "Goldene Statue")
        } else {
            t
        }
    };
    // Without the Statue player 0 has 10 points to player 1's 7: player 0 wins.
    let mut e = table(false).start();
    assert_eq!((score(&e, P0), score(&e, P1)), (10, 7));
    let castle = card(&e, P0, "Korallenschloss");
    use_power(&mut e, P0, castle, 0);
    assert_eq!(e.winners(), Some(&[P0][..]));
    // With it player 1 reaches 8 + 3 = 11 and takes the win.
    let mut e = table(true).start();
    let castle = card(&e, P0, "Korallenschloss");
    use_power(&mut e, P0, castle, 0);
    assert_eq!(e.pending().unwrap().players, [P1]);
    use_statue(&mut e);
    assert_eq!(e.winners(), Some(&[P1][..]));
}

#[test]
fn ties_at_the_victory_check_go_to_the_most_essences_with_gold_counting_double() {
    let table = |p1_pool: &[Qty]| {
        let mut t = Table::actions().pool(P0, &[(E, 1), (L, 1)]).pool(P1, p1_pool);
        for name in ["Korallenschloss", "Große Pyramide", "Orakel", "Tempel"] {
            t = t.card(P0, name);
        }
        for name in [
            "Koloss",
            "Mausoleum",
            "Obelisk",
            "Salomos Schatzkammer",
            "Hängende Gärten",
            "Bibliothek",
            "Nachtigall",
            "Wasserdrache",
        ] {
            t = t.card(P1, name);
        }
        t
    };
    let check = |p1_pool: &[Qty]| {
        let mut e = table(p1_pool).start();
        assert_eq!((score(&e, P0), score(&e, P1)), (10, 10));
        let castle = card(&e, P0, "Korallenschloss");
        use_power(&mut e, P0, castle, 0);
        e.winners().map(<[PlayerId]>::to_vec)
    };
    assert_eq!(
        check(&[(G, 2)]),
        Some(vec![P1]),
        "2 Gold count 4 against 2 essences"
    );
    assert_eq!(check(&[(C, 3)]), Some(vec![P1]), "3 essences against 2");
    assert_eq!(
        check(&[(G, 1)]),
        Some(vec![P0, P1]),
        "2 against 2: they share the victory"
    );
}

#[test]
fn the_first_player_token_is_worth_a_point_until_somebody_else_passes_first() {
    let mut e = Table::round().hand(P0, &["Prisma"]).hand(P1, &["Prisma"]).start();
    assert_eq!((first_token(&e, P0), first_token(&e, P1)), (0, 0));
    rival_passes(&mut e);
    assert_eq!(
        (score(&e, P0), score(&e, P1)),
        (0, 1),
        "player 1 passed first"
    );
    pass(&mut e, P0);
    // The next round: player 1 starts, the token stays with them.
    assert_eq!(e.state().flow.round, 2);
    assert_eq!(e.current_player(), Some(P1));
    assert_eq!(score(&e, P1), 1);
    // Now player 0 passes first and takes it away.
    rival_acts(&mut e);
    pass(&mut e, P0);
    assert_eq!((score(&e, P0), score(&e, P1)), (1, 0));
}

// ---------------------------------------------------------------------------------------------------
// Artifacts: dragons
// ---------------------------------------------------------------------------------------------------

/// Player 0's dragon uses its power on player 1 (who has 3 Life plus whatever
/// the table gives them).
fn dragon_vs(dragon: &str, rival: Table) -> Game {
    let mut e = rival.card(P0, dragon).start();
    let id = card(&e, P0, dragon);
    use_power(&mut e, P0, id, 0);
    assert!(is_turned(&e, id), "{dragon} turns to attack");
    e
}

/// The attack of a dragon on a rival who cannot (or does not) use the
/// dragon's escape: the rival loses 2 Life.
#[track_caller]
fn assert_dragon_takes_two_life(dragon: &str) {
    let e = dragon_vs(dragon, Table::actions().pool(P1, &[(L, 3)]));
    assert!(
        e.pending().is_none(),
        "{dragon}: nothing to decide without an escape"
    );
    assert_pool(&e, P1, &[(L, 1)]);
}

#[test]
fn bone_dragon_makes_rivals_lose_two_life_unless_they_pay_one_death() {
    assert_dragon_takes_two_life("Knochendrache");
    let rival = || Table::actions().pool(P1, &[(L, 3), (D, 1)]);
    // Take the loss.
    let mut e = dragon_vs("Knochendrache", rival());
    assert_eq!(options(&e).len(), 2);
    decide(&mut e, P1, 0);
    assert_pool(&e, P1, &[(L, 1), (D, 1)]);
    // Or pay the Death instead.
    let mut e = dragon_vs("Knochendrache", rival());
    decide_where(&mut e, P1, |o| {
        matches!(o, Effect::Pay { res: DEATH, n: 1 })
    });
    assert_pool(&e, P1, &[(L, 3)]);
}

#[test]
fn earth_dragon_lets_rivals_pay_one_gold_to_ignore_the_loss() {
    assert_dragon_takes_two_life("Erddrache");
    let mut e = dragon_vs("Erddrache", Table::actions().pool(P1, &[(L, 3), (G, 1)]));
    decide_where(&mut e, P1, |o| matches!(o, Effect::Pay { res: GOLD, n: 1 }));
    assert_pool(&e, P1, &[(L, 3)]);
}

#[test]
fn water_dragon_lets_rivals_pay_one_elan_to_ignore_the_loss() {
    assert_dragon_takes_two_life("Wasserdrache");
    let mut e = dragon_vs("Wasserdrache", Table::actions().pool(P1, &[(L, 3), (E, 1)]));
    decide_where(&mut e, P1, |o| matches!(o, Effect::Pay { res: ELAN, n: 1 }));
    assert_pool(&e, P1, &[(L, 3)]);
}

#[test]
fn fire_dragon_lets_rivals_pay_one_calm_to_ignore_the_loss() {
    assert_dragon_takes_two_life("Feuerdrache");
    let mut e = dragon_vs("Feuerdrache", Table::actions().pool(P1, &[(L, 3), (C, 1)]));
    decide_where(&mut e, P1, |o| matches!(o, Effect::Pay { res: CALM, n: 1 }));
    assert_pool(&e, P1, &[(L, 3)]);
}

#[test]
fn wind_dragon_lets_rivals_discard_a_card_to_ignore_the_loss() {
    assert_dragon_takes_two_life("Winddrache");
    let table = || Table::actions().pool(P1, &[(L, 3)]).hand(P1, &["Prisma", "Wachhund"]);
    let mut e = dragon_vs("Winddrache", table());
    assert_eq!(options(&e).len(), 2);
    decide_where(&mut e, P1, |o| matches!(o, Effect::DiscardChoose { n: 1 }));
    // The rival picks which card to discard.
    let dog = find(&e, P1, HAND, "Wachhund");
    discard_pick(&mut e, P1, dog);
    assert_pool(&e, P1, &[(L, 3)]);
    assert_eq!(names(&e, P1, HAND), ["Prisma"]);
    assert_eq!(names(&e, P1, DISCARD), ["Wachhund"]);
    // Declining costs the Life and keeps both cards.
    let mut e = dragon_vs("Winddrache", table());
    decide(&mut e, P1, 0);
    assert_pool(&e, P1, &[(L, 1)]);
    assert_eq!(names(&e, P1, HAND).len(), 2);
}

#[test]
fn sea_serpent_lets_rivals_destroy_one_of_their_artifacts_to_ignore_the_loss() {
    assert_dragon_takes_two_life("Seeschlange");
    let table = || {
        Table::actions().pool(P1, &[(L, 3)]).card(P1, "Nachtigall").loaded(
            P1,
            "Meerjungfrau",
            &[(C, 2)],
        )
    };
    let mut e = dragon_vs("Seeschlange", table());
    assert_eq!(
        options(&e).len(),
        2,
        "lose the Life, or destroy an artifact"
    );
    decide_where(&mut e, P1, |o| matches!(o, Effect::Pick { .. }));
    // The rival picks which artifact.
    assert_eq!(options(&e).len(), 2);
    let mermaid = card(&e, P1, "Meerjungfrau");
    decide_where(
        &mut e,
        P1,
        |o| matches!(o, Effect::WithCard { card, .. } if *card == mermaid),
    );
    assert_pool(&e, P1, &[(L, 3)]);
    assert!(has(&e, P1, DISCARD, "Meerjungfrau"));
    assert!(has(&e, P1, TABLEAU, "Nachtigall"));
}

#[test]
fn a_rival_with_too_little_life_loses_two_other_essences_for_each_missing_life() {
    // The Water Dragon takes 2 Life; the rival has 1, so 1 Life and 2 other
    // essences (of their choice, Gold included) go.
    let mut e = dragon_vs(
        "Wasserdrache",
        Table::actions().pool(P1, &[(L, 1), (C, 2), (G, 1)]),
    );
    assert_eq!(options(&e).len(), 2, "two Calm, or Calm and Gold");
    decide_where(&mut e, P1, |o| {
        gains_paid(o) == Some(amounts(&[(G, 1), (C, 1)]))
    });
    assert_pool(&e, P1, &[(C, 1)]);
}

/// The essences a lose option pays.
fn gains_paid(effect: &Effect) -> Option<BTreeMap<ResourceId, u32>> {
    sum_leaves(effect, &|leaf| match leaf {
        Effect::Pay { res, n } => Some((*res, *n)),
        _ => None,
    })
}

// ---------------------------------------------------------------------------------------------------
// Artifacts: powers
// ---------------------------------------------------------------------------------------------------

#[test]
fn jewel_statuette_makes_death_for_everybody_and_can_be_destroyed_for_gold() {
    let table = || Table::actions().card(P0, "Juwelenstatuette");
    let mut e = table().start();
    let statuette = card(&e, P0, "Juwelenstatuette");
    assert_eq!(score(&e, P0), 1);
    use_power(&mut e, P0, statuette, 0);
    assert_pool(&e, P0, &[(D, 3)]);
    assert_pool(&e, P1, &[(D, 1)]);
    assert!(is_turned(&e, statuette));
    // A turned component's powers cannot be used (rulebook p.7).
    let e = Table::actions().turned(P0, "Juwelenstatuette").start();
    assert!(!can_use(&e, P0, card(&e, P0, "Juwelenstatuette"), 1));
    // Destroying it gives 2 Gold and 1 Elan.
    let mut e = table().start();
    let statuette = card(&e, P0, "Juwelenstatuette");
    use_power(&mut e, P0, statuette, 1);
    assert_pool(&e, P0, &[(G, 2), (E, 1)]);
    assert!(has(&e, P0, DISCARD, "Juwelenstatuette"));
    assert_eq!(score(&e, P0), 0, "it no longer scores");
}

#[test]
fn corrupt_altar_collects_life_and_death_and_puts_elan_or_destroys_for_essences() {
    assert_collects("Verdorbener Altar", &[(L, 1), (D, 1)]);
    // 2 Life put 3 Elan on the Altar; the Altar stays straight.
    let mut e = Table::actions().card(P0, "Verdorbener Altar").pool(P0, &[(L, 3)]).start();
    let altar = card(&e, P0, "Verdorbener Altar");
    use_power(&mut e, P0, altar, 0);
    assert_eq!(on_card(&e, altar, E), 3);
    assert_pool(&e, P0, &[(L, 1)]);
    assert!(!is_turned(&e, altar));
    // Destroying the Mermaid (cost 2 Life and 2 Calm = 4) gives 4 + 2 essences.
    let mut e = Table::actions()
        .card(P0, "Verdorbener Altar")
        .loaded(P0, "Meerjungfrau", &[(C, 2)])
        .start();
    let (altar, mermaid) = (
        card(&e, P0, "Verdorbener Altar"),
        card(&e, P0, "Meerjungfrau"),
    );
    assert_eq!(
        power_payments(&e, P0, altar, 1).len(),
        2,
        "the Mermaid or the Altar"
    );
    use_power_where(&mut e, P0, altar, 1, |p| p.destroyed == Some(mermaid));
    assert_eq!(options(&e).len(), 84, "any mix of 6, no Gold");
    assert!(gain_options(&e).iter().all(|c| !c.contains_key(&G)));
    decide_gain(&mut e, P0, &[(E, 3), (L, 3)]);
    assert_pool(&e, P0, &[(E, 3), (L, 3)]);
    assert!(has(&e, P0, DISCARD, "Meerjungfrau"));
    assert!(is_turned(&e, altar));
    // The Altar may destroy itself: its own cost is 3 any + 2 Death = 5, so 7.
    let mut e = Table::actions().card(P0, "Verdorbener Altar").start();
    let altar = card(&e, P0, "Verdorbener Altar");
    use_power(&mut e, P0, altar, 1);
    assert_eq!(options(&e).len(), 120, "any mix of 7 out of 4 kinds");
    decide_gain(&mut e, P0, &[(D, 7)]);
    assert_pool(&e, P0, &[(D, 7)]);
    assert!(has(&e, P0, DISCARD, "Verdorbener Altar"));
}

#[test]
fn mermaid_collects_calm_and_puts_a_paid_essence_on_any_component() {
    assert_collects("Meerjungfrau", &[(C, 1)]);
    let table = || {
        Table::actions()
            .card(P0, "Meerjungfrau")
            .card(P0, "Verfluchte Schmiede")
            .pool(P0, &[(C, 1), (L, 1), (G, 1), (E, 1)])
    };
    let mut e = table().start();
    let (mermaid, forge) = (
        card(&e, P0, "Meerjungfrau"),
        card(&e, P0, "Verfluchte Schmiede"),
    );
    assert_eq!(
        power_payments(&e, P0, mermaid, 0).len(),
        3,
        "Calm, Life or Gold, not Elan"
    );
    use_power_paying(&mut e, P0, mermaid, 0, &[(G, 1)]);
    assert_eq!(options(&e).len(), 2, "either component");
    decide_where(
        &mut e,
        P0,
        |o| matches!(o, Effect::WithCard { card, .. } if *card == forge),
    );
    assert_eq!(on_card(&e, forge, G), 1, "the paid Gold went on the Forge");
    assert_pool(&e, P0, &[(C, 1), (L, 1), (E, 1)]);
    assert!(is_turned(&e, mermaid));
    // It can also go on the Mermaid itself, or on a turned component.
    let mut e = table().turned(P0, "Wachhund").start();
    let (mermaid, dog) = (card(&e, P0, "Meerjungfrau"), card(&e, P0, "Wachhund"));
    use_power_paying(&mut e, P0, mermaid, 0, &[(L, 1)]);
    assert_eq!(options(&e).len(), 3);
    decide_where(
        &mut e,
        P0,
        |o| matches!(o, Effect::WithCard { card, .. } if *card == dog),
    );
    assert_eq!(on_card(&e, dog, L), 1);
}

#[test]
fn hand_of_glory_gives_two_death_and_every_rival_one_even_after_passing() {
    let mut e = Table::actions().card(P0, "Hand des Ruhmes").hand(P0, &["Prisma"]).start();
    let hand = card(&e, P0, "Hand des Ruhmes");
    let gold_before = pool(&e, P0, G);
    rival_passes(&mut e);
    use_power(&mut e, P0, hand, 0);
    assert_eq!(pool(&e, P0, D), 2);
    assert_eq!(
        pool(&e, P0, G),
        gold_before + 1,
        "only the discard gave Gold"
    );
    assert_eq!(pool(&e, P1, D), 1, "player 1 passed and still gains");
    assert!(is_turned(&e, hand));
}

#[test]
fn dragon_tooth_makes_elan_and_places_a_dragon_from_hand_for_free() {
    let mut e = Table::actions().card(P0, "Drachenzahn").pool(P0, &[(E, 2)]).start();
    let tooth = card(&e, P0, "Drachenzahn");
    use_power(&mut e, P0, tooth, 0);
    assert_eq!(on_card(&e, tooth, E), 3);
    assert_pool(&e, P0, &[]);
    assert!(!is_turned(&e, tooth));
    // Turning with 3 Elan places any Dragon of the hand; others are not
    // offered.
    let table = || {
        Table::actions()
            .card(P0, "Drachenzahn")
            .pool(P0, &[(E, 3)])
            .hand(P0, &["Erddrache", "Feuerdrache", "Nachtigall"])
    };
    let mut e = table().start();
    let tooth = card(&e, P0, "Drachenzahn");
    use_power(&mut e, P0, tooth, 1);
    assert_eq!(options(&e).len(), 3, "two Dragons, or none");
    let fire = find(&e, P0, HAND, "Feuerdrache");
    decide_place(&mut e, P0, fire);
    assert!(has(&e, P0, TABLEAU, "Feuerdrache"));
    assert_eq!(names(&e, P0, HAND), ["Erddrache", "Nachtigall"]);
    assert_pool(&e, P0, &[]);
    assert!(is_turned(&e, tooth));
    // Once the cost is paid a Dragon must be placed: declining is not legal.
    let mut e = table().start();
    let tooth = card(&e, P0, "Drachenzahn");
    use_power(&mut e, P0, tooth, 1);
    let decline = options(&e).iter().position(|o| *o == Effect::Ignore).expect("listed last");
    assert!(!e.legal_actions(P0).contains(&Action::Decide(decline)));
    assert_eq!(e.legal_actions(P0).len(), 2, "only the two Dragons");
    // Without a Dragon in hand the power cannot be used at all.
    let e = Table::actions()
        .card(P0, "Drachenzahn")
        .pool(P0, &[(E, 3)])
        .hand(P0, &["Nachtigall"])
        .start();
    assert!(!can_use(&e, P0, card(&e, P0, "Drachenzahn"), 1));
    // Its other power is gone too once the Tooth is turned.
    let e = Table::actions().turned(P0, "Drachenzahn").pool(P0, &[(E, 2)]).start();
    assert!(!can_use(&e, P0, card(&e, P0, "Drachenzahn"), 0));
}

/// Picks "place this card" among the options of a placing power.
#[track_caller]
fn decide_place(e: &mut Game, p: PlayerId, id: EntityId) {
    decide_where(e, p, |o| {
        matches!(o, Effect::Sequence(steps)
            if steps.iter().any(|s| matches!(s, Effect::WithCard { card, .. } if *card == id)))
    });
}

#[test]
fn elemental_source_collects_three_essences_and_pays_calm_even_while_turned() {
    assert_collects("Elementarquelle", &[(C, 1), (L, 1), (E, 1)]);
    // The React costs 1 Calm, not a turn, so a turned Source can use it.
    let mut e = bow_vs(Table::actions().turned(P1, "Elementarquelle").pool(P1, &[(L, 3), (C, 1)]));
    let source = card(&e, P1, "Elementarquelle");
    assert_eq!(options(&e).len(), 2);
    react_with(&mut e, source);
    assert_pool(&e, P1, &[(L, 3)]);
    assert!(is_turned(&e, source), "it stays turned");
    // Without the Calm it cannot.
    let e = bow_vs(Table::actions().card(P1, "Elementarquelle").pool(P1, &[(L, 3)]));
    assert!(e.pending().is_none());
    assert_pool(&e, P1, &[(L, 2)]);
}

#[test]
fn dwarven_pickaxe_costs_one_elan_and_turns_with_elan_for_gold() {
    let mut e = Table::actions()
        .hand(P0, &["Zwergen-Spitzhacke"])
        .hand(P1, &["Prisma"])
        .pool(P0, &[(E, 2)])
        .start();
    place(&mut e, P0, "Zwergen-Spitzhacke", &[(E, 1)]);
    rival_acts(&mut e);
    let pickaxe = card(&e, P0, "Zwergen-Spitzhacke");
    assert!(!is_turned(&e, pickaxe), "it begins play straightened");
    use_power(&mut e, P0, pickaxe, 0);
    assert_pool(&e, P0, &[(G, 1)]);
}

#[test]
fn fiery_bow_takes_one_life_from_rivals_and_draws_a_card() {
    // 1 Life lost.
    let e = bow_vs(Table::actions().pool(P1, &[(L, 2)]));
    assert_pool(&e, P1, &[(L, 1)]);
    // Both powers turn the Bow: one of them per round.
    let bow = card(&e, P0, "Elfenbogen");
    assert!(is_turned(&e, bow));
    assert!(!can_use(&e, P0, bow, 1));
    let mut e = Table::actions().card(P0, "Elfenbogen").deck(P0, &["Prisma"]).start();
    let bow = card(&e, P0, "Elfenbogen");
    use_power(&mut e, P0, bow, 1);
    assert_eq!(names(&e, P0, HAND), ["Prisma"]);
    assert!(!can_use(&e, P0, bow, 0));
    // A rival without Life loses 2 other essences of their choice.
    let mut e = bow_vs(Table::actions().pool(P1, &[(E, 1), (C, 1), (G, 1)]));
    assert_eq!(options(&e).len(), 3);
    decide_where(&mut e, P1, |o| {
        gains_paid(o) == Some(amounts(&[(G, 1), (C, 1)]))
    });
    assert_pool(&e, P1, &[(E, 1)]);
}

#[test]
fn a_passed_rival_ignores_life_loss() {
    let mut e =
        Table::actions().card(P0, "Elfenbogen").hand(P0, &["Prisma"]).pool(P1, &[(L, 2)]).start();
    rival_passes(&mut e);
    let bow = card(&e, P0, "Elfenbogen");
    use_power(&mut e, P0, bow, 0);
    assert_eq!(pool(&e, P1, L), 2);
    assert!(e.pending().is_none());
}

#[test]
fn fiery_whip_makes_elan_for_everybody_or_destroys_another_artifact_for_essences() {
    let mut e = Table::actions().card(P0, "Flammenpeitsche").start();
    let whip = card(&e, P0, "Flammenpeitsche");
    assert!(!can_use(&e, P0, whip, 1), "it cannot destroy itself");
    use_power(&mut e, P0, whip, 0);
    assert_pool(&e, P0, &[(E, 3)]);
    assert_pool(&e, P1, &[(E, 1)]);
    // Destroy the Nightingale (cost 2): 2 + 2 = 4 essences, no Gold.
    let mut e = Table::actions().card(P0, "Flammenpeitsche").card(P0, "Nachtigall").start();
    let whip = card(&e, P0, "Flammenpeitsche");
    assert_eq!(power_payments(&e, P0, whip, 1).len(), 1);
    use_power(&mut e, P0, whip, 1);
    assert_eq!(options(&e).len(), 35, "any mix of 4 out of 4 kinds");
    assert!(gain_options(&e).iter().all(|c| !c.contains_key(&G)));
    decide_gain(&mut e, P0, &[(C, 4)]);
    assert_pool(&e, P0, &[(C, 4)]);
    assert!(has(&e, P0, DISCARD, "Nachtigall"));
    assert!(is_turned(&e, whip));
}

#[test]
fn athanor_builds_elan_on_itself_and_turns_in_six_of_them_for_gold() {
    let mut e = Table::actions().card(P0, "Athanor").pool(P0, &[(E, 1)]).start();
    let athanor = card(&e, P0, "Athanor");
    use_power(&mut e, P0, athanor, 0);
    assert_eq!(on_card(&e, athanor, E), 2);
    assert_pool(&e, P0, &[]);
    assert!(is_turned(&e, athanor));
    // 6 Elan from the card and N identical essences from the pool: N Gold.
    let table = |on_it: u32| {
        Table::actions().loaded(P0, "Athanor", &[(E, on_it)]).pool(P0, &[(C, 3), (L, 1)])
    };
    let mut e = table(8).start();
    let athanor = card(&e, P0, "Athanor");
    assert_eq!(
        power_payments(&e, P0, athanor, 1).len(),
        4,
        "1 to 3 Calm, or 1 Life"
    );
    use_power_where(&mut e, P0, athanor, 1, |p| {
        p.resources == amounts(&[(C, 2)])
    });
    assert_eq!(on_card(&e, athanor, E), 2, "6 of the 8 Elan were turned in");
    assert_pool(&e, P0, &[(C, 1), (L, 1), (G, 2)]);
    assert!(is_turned(&e, athanor));
    // 5 Elan are not enough.
    let e = table(5).start();
    assert!(!can_use(&e, P0, card(&e, P0, "Athanor"), 1));
}

#[test]
fn cornucopia_has_two_powers_that_each_turn_it() {
    let mut e = Table::actions().card(P0, "Füllhorn").start();
    let horn = card(&e, P0, "Füllhorn");
    use_power(&mut e, P0, horn, 0);
    assert_eq!(options(&e).len(), 20);
    assert!(gain_options(&e).iter().all(|c| !c.contains_key(&G)));
    decide_gain(&mut e, P0, &[(E, 1), (L, 1), (D, 1)]);
    assert_pool(&e, P0, &[(E, 1), (L, 1), (D, 1)]);
    assert!(!can_use(&e, P0, horn, 1));
    let mut e = Table::actions().card(P0, "Füllhorn").start();
    let horn = card(&e, P0, "Füllhorn");
    use_power(&mut e, P0, horn, 1);
    assert_pool(&e, P0, &[(G, 1)]);
    assert!(is_turned(&e, horn));
}

#[test]
fn guard_dog_straightens_itself_for_elan_even_while_turned_and_ignores_life_loss() {
    assert_turns_to_ignore_life_loss("Wachhund");
    let mut e = Table::actions().turned(P0, "Wachhund").pool(P0, &[(E, 1), (C, 1)]).start();
    let dog = card(&e, P0, "Wachhund");
    assert!(can_use(&e, P0, dog, 0), "usable while turned");
    use_power(&mut e, P0, dog, 0);
    assert!(!is_turned(&e, dog));
    assert_pool(&e, P0, &[(C, 1)]);
}

#[test]
fn magical_shard_is_free_to_place_and_makes_one_essence_but_gold() {
    let mut e = Table::actions().hand(P0, &["Magische Scherbe"]).start();
    place(&mut e, P0, "Magische Scherbe", &[]);
    assert_eq!(
        e.current_player(),
        Some(P1),
        "placing it is still an action"
    );
    assert!(has(&e, P0, TABLEAU, "Magische Scherbe"));
    assert_pool(&e, P0, &[]);
    let mut e = Table::actions().card(P0, "Magische Scherbe").start();
    let shard = card(&e, P0, "Magische Scherbe");
    use_power(&mut e, P0, shard, 0);
    assert_eq!(options(&e).len(), 4);
    assert!(gain_options(&e).iter().all(|c| !c.contains_key(&G)));
    decide_gain(&mut e, P0, &[(D, 1)]);
    assert_pool(&e, P0, &[(D, 1)]);
    assert!(is_turned(&e, shard));
}

#[test]
fn prism_makes_two_essences_for_one_and_converts_identical_essences() {
    let mut e = Table::actions().hand(P0, &["Prisma"]).start();
    place(&mut e, P0, "Prisma", &[]);
    assert!(has(&e, P0, TABLEAU, "Prisma"));
    let mut e = Table::actions().card(P0, "Prisma").pool(P0, &[(E, 1)]).start();
    let prism = card(&e, P0, "Prisma");
    use_power_paying(&mut e, P0, prism, 0, &[(E, 1)]);
    assert_eq!(options(&e).len(), 10);
    assert!(gain_options(&e).iter().all(|c| !c.contains_key(&G)));
    decide_gain(&mut e, P0, &[(L, 1), (C, 1)]);
    assert_pool(&e, P0, &[(L, 1), (C, 1)]);
    assert!(is_turned(&e, prism));
    // N identical essences become N identical essences (never Gold).
    let mut e = Table::actions().card(P0, "Prisma").pool(P0, &[(C, 3), (E, 1)]).start();
    let prism = card(&e, P0, "Prisma");
    assert_eq!(
        power_payments(&e, P0, prism, 1).len(),
        4,
        "1 to 3 Calm, or 1 Elan"
    );
    use_power_where(&mut e, P0, prism, 1, |p| p.resources == amounts(&[(C, 3)]));
    assert!(gain_options(&e).iter().all(|c| !c.contains_key(&G) && c.values().sum::<u32>() == 3));
    decide_gain(&mut e, P0, &[(L, 3)]);
    assert_pool(&e, P0, &[(L, 3), (E, 1)]);
}

#[test]
fn sacrificial_dagger_makes_death_on_itself_or_trades_a_discarded_card_for_essences() {
    let mut e = Table::actions().card(P0, "Opferdolch").pool(P0, &[(L, 1)]).start();
    let dagger = card(&e, P0, "Opferdolch");
    use_power(&mut e, P0, dagger, 0);
    assert_eq!(on_card(&e, dagger, D), 3);
    assert_pool(&e, P0, &[]);
    assert!(is_turned(&e, dagger));
    // Destroy it and discard the Hand of Glory (cost Life and Death = 2): 2
    // essences, not Gold; the discard gives nothing else.
    let table = || Table::actions().card(P0, "Opferdolch").hand(P0, &["Hand des Ruhmes"]);
    let mut e = table().start();
    let dagger = card(&e, P0, "Opferdolch");
    use_power(&mut e, P0, dagger, 1);
    assert_eq!(options(&e).len(), 10);
    assert!(gain_options(&e).iter().all(|c| !c.contains_key(&G)));
    decide_gain(&mut e, P0, &[(D, 1), (C, 1)]);
    assert_pool(&e, P0, &[(D, 1), (C, 1)]);
    assert!(has(&e, P0, DISCARD, "Opferdolch") && has(&e, P0, DISCARD, "Hand des Ruhmes"));
    assert_eq!(names(&e, P0, HAND).len(), 0);
    // It needs a card in hand, and a turned Dagger has no powers.
    let e = Table::actions().card(P0, "Opferdolch").start();
    assert!(!can_use(&e, P0, card(&e, P0, "Opferdolch"), 1));
    let e = Table::actions().turned(P0, "Opferdolch").hand(P0, &["Prisma"]).start();
    assert!(!can_use(&e, P0, card(&e, P0, "Opferdolch"), 1));
    // A free card is worth nothing.
    let mut e = Table::actions().card(P0, "Opferdolch").hand(P0, &["Prisma"]).start();
    let dagger = card(&e, P0, "Opferdolch");
    use_power(&mut e, P0, dagger, 1);
    assert!(e.pending().is_none());
    assert_pool(&e, P0, &[]);
}

#[test]
fn tree_of_life_makes_life_for_everybody_and_pays_life_to_ignore_life_loss() {
    let mut e = Table::actions().card(P0, "Baum des Lebens").start();
    let tree = card(&e, P0, "Baum des Lebens");
    use_power(&mut e, P0, tree, 0);
    assert_pool(&e, P0, &[(L, 3)]);
    assert_pool(&e, P1, &[(L, 1)]);
    // The Bone Dragon takes 2 Life, or the rival pays 1 Life with the React.
    let table = || {
        Table::actions().card(P0, "Knochendrache").card(P1, "Baum des Lebens").pool(P1, &[(L, 3)])
    };
    let attack = |mut e: Game| {
        let dragon = card(&e, P0, "Knochendrache");
        use_power(&mut e, P0, dragon, 0);
        e
    };
    let mut e = attack(table().start());
    let tree = card(&e, P1, "Baum des Lebens");
    assert_eq!(options(&e).len(), 2);
    react_with(&mut e, tree);
    assert_pool(&e, P1, &[(L, 2)]);
    assert!(!is_turned(&e, tree), "paying Life does not turn it");
    let mut e = attack(table().start());
    decide(&mut e, P1, 0);
    assert_pool(&e, P1, &[(L, 1)]);
}

#[test]
fn hawk_collects_calm_looks_at_three_cards_or_draws_for_two_calm() {
    assert_collects("Falke", &[(C, 1)]);
    let table =
        || Table::actions().card(P0, "Falke").deck(P0, &["Prisma", "Wachhund", "Nachtigall"]);
    let mut e = table().start();
    let hawk = card(&e, P0, "Falke");
    use_power(&mut e, P0, hawk, 0);
    peek(&mut e, DECK, &[1, 0, 2]);
    assert_eq!(
        top_names(&e, &Owner::Player(P0), DECK, 3),
        ["Wachhund", "Prisma", "Nachtigall"]
    );
    assert!(!can_use(&e, P0, hawk, 1), "both powers turn the Hawk");
    let mut e = table().pool(P0, &[(C, 2), (E, 1)]).start();
    let hawk = card(&e, P0, "Falke");
    use_power(&mut e, P0, hawk, 1);
    assert_eq!(names(&e, P0, HAND), ["Prisma"]);
    assert_pool(&e, P0, &[(E, 1)]);
    assert!(is_turned(&e, hawk));
}

#[test]
fn chalice_of_fire_collects_two_elan_and_straightens_any_component() {
    assert_collects("Flammenkelch", &[(E, 2)]);
    let mut e = Table::actions()
        .card(P0, "Flammenkelch")
        .turned(P0, "Wachhund")
        .pool(P0, &[(E, 1), (L, 1)])
        .start();
    let (chalice, dog) = (card(&e, P0, "Flammenkelch"), card(&e, P0, "Wachhund"));
    use_power_paying(&mut e, P0, chalice, 0, &[(E, 1)]);
    assert_eq!(options(&e).len(), 2, "the dog or the turned Chalice itself");
    decide_where(
        &mut e,
        P0,
        |o| matches!(o, Effect::WithCard { card, .. } if *card == dog),
    );
    assert!(!is_turned(&e, dog));
    assert!(is_turned(&e, chalice));
    assert_pool(&e, P0, &[(L, 1)]);
}

#[test]
fn chalice_of_life_collects_calm_and_life_puts_life_on_itself_and_ignores_life_loss() {
    assert_collects("Kelch des Lebens", &[(C, 1), (L, 1)]);
    assert_turns_to_ignore_life_loss("Kelch des Lebens");
    let mut e = Table::actions().card(P0, "Kelch des Lebens").pool(P0, &[(C, 2)]).start();
    let chalice = card(&e, P0, "Kelch des Lebens");
    use_power(&mut e, P0, chalice, 0);
    assert_eq!(on_card(&e, chalice, L), 1);
    assert_eq!(
        pool(&e, P0, C) + on_card(&e, chalice, C),
        2,
        "2 Calm paid, 2 Calm put back"
    );
    assert!(!is_turned(&e, chalice));
}

#[test]
fn chalice_of_life_puts_both_the_calm_and_the_life_on_itself() {
    let mut e = Table::actions().card(P0, "Kelch des Lebens").pool(P0, &[(C, 2)]).start();
    let chalice = card(&e, P0, "Kelch des Lebens");
    use_power(&mut e, P0, chalice, 0);
    assert_eq!((on_card(&e, chalice, C), on_card(&e, chalice, L)), (2, 1));
    assert_pool(&e, P0, &[]);
}

#[test]
fn celestial_horse_collects_two_essences_of_elan_life_or_calm() {
    let mut e = Table::round().card(P0, "Himmlisches Pferd").start();
    assert_eq!(options(&e).len(), 6, "every mix of 2 out of 3 kinds");
    assert!(
        gain_options(&e).iter().all(|c| !c.contains_key(&G) && !c.contains_key(&D)),
        "neither Gold nor Death"
    );
    decide_gain(&mut e, P0, &[(E, 1), (C, 1)]);
    assert_pool(&e, P0, &[(E, 1), (C, 1)]);
}

#[test]
fn philosophers_stone_turns_in_two_any_and_n_identical_essences_for_n_gold() {
    let mut e =
        Table::actions().card(P0, "Stein der Weisen").pool(P0, &[(L, 2), (E, 3), (D, 1)]).start();
    let stone = card(&e, P0, "Stein der Weisen");
    assert_eq!(score(&e, P0), 1);
    // 2 of any kind (Life) and 3 identical Elan: 3 Gold.
    use_power_where(&mut e, P0, stone, 0, |p| {
        p.resources == amounts(&[(L, 2), (E, 3)]) && p.x == Some(3)
    });
    assert_pool(&e, P0, &[(D, 1), (G, 3)]);
    assert!(is_turned(&e, stone));
    // The identical essences need at least 1, so 2 essences are not enough.
    let e = Table::actions().card(P0, "Stein der Weisen").pool(P0, &[(L, 2)]).start();
    assert!(!can_use(&e, P0, card(&e, P0, "Stein der Weisen"), 0));
}

#[test]
fn treant_collects_two_life_and_gains_elan_for_each_death_of_a_rival() {
    assert_collects("Baumhirte", &[(L, 2)]);
    let mut e = Table::actions().card(P0, "Baumhirte").pool(P1, &[(D, 3), (E, 5)]).start();
    let treant = card(&e, P0, "Baumhirte");
    use_power(&mut e, P0, treant, 0);
    assert_pool(&e, P0, &[(E, 3)]);
    assert_pool(&e, P1, &[(D, 3), (E, 5)]);
    assert!(is_turned(&e, treant));
    // A rival without Death gives nothing, but the power is used all the same.
    let mut e = Table::actions().card(P0, "Baumhirte").start();
    let treant = card(&e, P0, "Baumhirte");
    use_power(&mut e, P0, treant, 0);
    assert_pool(&e, P0, &[]);
    assert!(is_turned(&e, treant));
}

#[test]
fn nightingale_is_a_creature_worth_a_point_with_no_powers() {
    let e = Table::actions().card(P0, "Nachtigall").start();
    assert_eq!(score(&e, P0), 1);
    assert_eq!(abilities_of(&e, card(&e, P0, "Nachtigall")), 0);
    assert!(
        e.state()
            .defs
            .values()
            .any(|d| d.name == "Nachtigall" && d.tags.contains(&tag::CREATURE.to_owned()))
    );
}

#[test]
fn fountain_of_youth_collects_life_and_puts_life_on_itself_for_two_death() {
    assert_collects("Jungbrunnen", &[(L, 1)]);
    let mut e = Table::actions().card(P0, "Jungbrunnen").pool(P0, &[(D, 2)]).start();
    let fountain = card(&e, P0, "Jungbrunnen");
    use_power(&mut e, P0, fountain, 0);
    assert_eq!(on_card(&e, fountain, L), 1);
    assert_eq!(pool(&e, P0, D), 0);
    assert_eq!(pool(&e, P0, C) + on_card(&e, fountain, C), 2);
    assert!(!is_turned(&e, fountain));
}

#[test]
fn fountain_of_youth_puts_both_the_calm_and_the_life_on_itself() {
    let mut e = Table::actions().card(P0, "Jungbrunnen").pool(P0, &[(D, 2)]).start();
    let fountain = card(&e, P0, "Jungbrunnen");
    use_power(&mut e, P0, fountain, 0);
    assert_eq!((on_card(&e, fountain, C), on_card(&e, fountain, L)), (2, 1));
    assert_pool(&e, P0, &[]);
}

#[test]
fn dragon_egg_is_destroyed_to_place_a_dragon_from_hand_for_four_less() {
    let mut e = Table::actions()
        .card(P0, "Drachenei")
        .hand(P0, &["Wasserdrache", "Hand des Ruhmes"])
        .pool(P0, &[(C, 2)])
        .start();
    let egg = card(&e, P0, "Drachenei");
    assert_eq!(score(&e, P0), 1);
    use_power(&mut e, P0, egg, 0);
    assert!(has(&e, P0, DISCARD, "Drachenei"), "the egg is destroyed");
    assert_eq!(
        options(&e).len(),
        2,
        "the Water Dragon for 2 Calm, or nothing"
    );
    let dragon = find(&e, P0, HAND, "Wasserdrache");
    decide_place(&mut e, P0, dragon);
    assert!(has(&e, P0, TABLEAU, "Wasserdrache"));
    assert_pool(&e, P0, &[]);
    assert_eq!(score(&e, P0), 1, "the Dragon is worth 1, the egg no longer");
    // Without a Dragon in hand, or one that can be paid, the egg stays.
    let e = Table::actions().card(P0, "Drachenei").hand(P0, &["Hand des Ruhmes"]).start();
    assert!(!can_use(&e, P0, card(&e, P0, "Drachenei"), 0));
    let e = Table::actions().card(P0, "Drachenei").hand(P0, &["Wasserdrache"]).start();
    assert!(
        !can_use(&e, P0, card(&e, P0, "Drachenei"), 0),
        "2 Calm short"
    );
}

#[test]
fn vault_gains_interest_when_gold_is_left_on_it() {
    let table = || Table::round().loaded(P0, "Tresor", &[(G, 1)]);
    // Leave the Gold: 2 essences but Gold, and the Gold stays.
    let mut e = table().start();
    let vault = card(&e, P0, "Tresor");
    leave_essences(&mut e, P0);
    assert_eq!(options(&e).len(), 10);
    assert!(gain_options(&e).iter().all(|c| !c.contains_key(&G)));
    decide_gain(&mut e, P0, &[(E, 1), (L, 1)]);
    assert_pool(&e, P0, &[(E, 1), (L, 1)]);
    assert_eq!(on_card(&e, vault, G), 1);
    // Take the Gold: no interest.
    let mut e = table().start();
    let vault = card(&e, P0, "Tresor");
    take_from(&mut e, P0, vault);
    assert!(e.pending().is_none());
    assert_pool(&e, P0, &[(G, 1)]);
    assert_eq!(on_card(&e, vault, G), 0);
    // Nothing on it: no interest either, and its power puts 1 Gold on it.
    let mut e = Table::actions().card(P0, "Tresor").start();
    let vault = card(&e, P0, "Tresor");
    use_power(&mut e, P0, vault, 0);
    assert_eq!(on_card(&e, vault, G), 1);
    assert!(is_turned(&e, vault));
    assert_pool(&e, P0, &[]);
    let e = Table::round().card(P0, "Tresor").start();
    assert!(e.pending().is_none());
    assert_pool(&e, P0, &[]);
}

#[test]
fn flame_pit_collects_elan_and_turns_with_life_for_elan_and_death() {
    assert_collects("Flammengrube", &[(E, 1)]);
    let mut e = Table::actions().card(P0, "Flammengrube").pool(P0, &[(L, 1)]).start();
    let pit = card(&e, P0, "Flammengrube");
    use_power(&mut e, P0, pit, 0);
    assert_pool(&e, P0, &[(E, 1), (D, 1)]);
    assert!(is_turned(&e, pit));
}

#[test]
fn windup_man_grows_the_essences_left_on_it_and_stores_the_paid_essence() {
    let table = || Table::round().loaded(P0, "Aufziehmann", &[(G, 1), (E, 1)]);
    // Left on the card: 2 more of each kind present.
    let mut e = table().start();
    let man = card(&e, P0, "Aufziehmann");
    leave_essences(&mut e, P0);
    assert_eq!((on_card(&e, man, G), on_card(&e, man, E)), (3, 3));
    assert_pool(&e, P0, &[]);
    // Taken: they are in the pool and the card does not grow.
    let mut e = table().start();
    let man = card(&e, P0, "Aufziehmann");
    take_from(&mut e, P0, man);
    assert_eq!((on_card(&e, man, G), on_card(&e, man, E)), (0, 0));
    assert_pool(&e, P0, &[(G, 1), (E, 1)]);
    // The power: turn with 1 essence, which is put on the Windup Man.
    let mut e = Table::actions().card(P0, "Aufziehmann").pool(P0, &[(C, 1), (L, 1)]).start();
    let man = card(&e, P0, "Aufziehmann");
    use_power_paying(&mut e, P0, man, 0, &[(C, 1)]);
    assert_eq!(on_card(&e, man, C), 1);
    assert_pool(&e, P0, &[(L, 1)]);
    assert!(is_turned(&e, man));
}

#[test]
fn crypt_makes_death_or_places_an_artifact_from_the_discard_pile_for_less() {
    let mut e = Table::actions().card(P0, "Krypta").start();
    let crypt = card(&e, P0, "Krypta");
    use_power(&mut e, P0, crypt, 0);
    assert_pool(&e, P0, &[(D, 2)]);
    // The Dagger (Death and Gold) from the discard pile: 2 less, but never off
    // the Gold, so it costs just the Gold.
    let table = || {
        Table::actions()
            .card(P0, "Krypta")
            .hand(P0, &["Nachtigall"])
            .discard(P0, &["Opferdolch"])
            .pool(P0, &[(D, 1), (G, 1)])
    };
    let mut e = table().start();
    let crypt = card(&e, P0, "Krypta");
    use_power(&mut e, P0, crypt, 1);
    assert_eq!(
        options(&e).len(),
        2,
        "only the discarded Dagger, or nothing"
    );
    let dagger = find(&e, P0, DISCARD, "Opferdolch");
    decide_place(&mut e, P0, dagger);
    assert!(has(&e, P0, TABLEAU, "Opferdolch"));
    assert_eq!(
        names(&e, P0, HAND),
        ["Nachtigall"],
        "the hand is not touched"
    );
    assert_pool(&e, P0, &[]);
    assert!(is_turned(&e, crypt));
    // Without the Gold nothing could be placed, so the power cannot be used.
    let e = table().pool(P0, &[(D, 1)]).start();
    assert!(!can_use(&e, P0, card(&e, P0, "Krypta"), 1));
}

#[test]
fn hypnotic_basin_collects_two_calm_and_gains_calm_for_each_elan_of_a_rival() {
    assert_collects("Hypnotisches Becken", &[(C, 2)]);
    let mut e =
        Table::actions().card(P0, "Hypnotisches Becken").pool(P1, &[(E, 2), (D, 4)]).start();
    let basin = card(&e, P0, "Hypnotisches Becken");
    use_power(&mut e, P0, basin, 0);
    assert_pool(&e, P0, &[(C, 2)]);
    assert_pool(&e, P1, &[(E, 2), (D, 4)]);
    assert!(is_turned(&e, basin));
}

#[test]
fn cursed_skull_turns_with_life_to_put_three_essences_on_itself() {
    let mut e = Table::actions().card(P0, "Verfluchter Schädel").pool(P0, &[(L, 1)]).start();
    let skull = card(&e, P0, "Verfluchter Schädel");
    use_power(&mut e, P0, skull, 0);
    assert_eq!(
        options(&e).len(),
        10,
        "every mix of 3 of Elan, Calm and Death"
    );
    assert!(gain_options(&e).iter().all(|c| !c.contains_key(&G) && !c.contains_key(&L)));
    decide_gain(&mut e, P0, &[(D, 2), (E, 1)]);
    assert_eq!((on_card(&e, skull, D), on_card(&e, skull, E)), (2, 1));
    assert_pool(&e, P0, &[]);
    assert!(is_turned(&e, skull));
}

#[test]
fn dancing_sword_collects_death_and_elan_and_pays_elan_to_ignore_life_loss() {
    assert_collects("Tanzendes Schwert", &[(D, 1), (E, 1)]);
    let mut e = bow_vs(Table::actions().card(P1, "Tanzendes Schwert").pool(P1, &[(L, 3), (E, 1)]));
    let sword = card(&e, P1, "Tanzendes Schwert");
    assert_eq!(options(&e).len(), 2);
    react_with(&mut e, sword);
    assert_pool(&e, P1, &[(L, 3)]);
    assert_eq!(
        on_card(&e, sword, D),
        1,
        "the reaction also puts 1 Death on the Sword"
    );
    assert!(!is_turned(&e, sword));
}

#[test]
fn ring_of_midas_puts_gold_on_itself_for_two_life_or_by_turning() {
    let mut e =
        Table::actions().card(P0, "Midas Ring").pool(P0, &[(L, 4)]).hand(P1, &["Prisma"]).start();
    let ring = card(&e, P0, "Midas Ring");
    assert_eq!(score(&e, P0), 1);
    use_power(&mut e, P0, ring, 0);
    assert_eq!(on_card(&e, ring, G), 1);
    assert_pool(&e, P0, &[(L, 2)]);
    assert!(!is_turned(&e, ring), "this power does not turn the Ring");
    rival_acts(&mut e);
    use_power(&mut e, P0, ring, 0);
    assert_eq!(on_card(&e, ring, G), 2);
    assert_pool(&e, P0, &[]);
    // Turning is free.
    let mut e = Table::actions().card(P0, "Midas Ring").start();
    let ring = card(&e, P0, "Midas Ring");
    use_power(&mut e, P0, ring, 1);
    assert_eq!(on_card(&e, ring, G), 1);
    assert!(is_turned(&e, ring));
}

// ---------------------------------------------------------------------------------------------------
// Costs and points of every card
// ---------------------------------------------------------------------------------------------------

// (name, specific essences, number of any-essence units): the placement cost
// of every artifact, as printed.
type ArtifactCost = (&'static str, &'static [Qty], u32);

const ARTIFACT_COSTS: [ArtifactCost; 40] = [
    ("Knochendrache", &[(D, 4), (L, 1)], 0),
    ("Seeschlange", &[(C, 6), (L, 3)], 0),
    ("Juwelenstatuette", &[(D, 2), (G, 1)], 0),
    ("Verdorbener Altar", &[(D, 2)], 3),
    ("Meerjungfrau", &[(L, 2), (C, 2)], 0),
    ("Hand des Ruhmes", &[(L, 1), (D, 1)], 0),
    ("Drachenzahn", &[(E, 1), (D, 1)], 0),
    ("Elementarquelle", &[(E, 2), (L, 1), (C, 1)], 0),
    ("Zwergen-Spitzhacke", &[(E, 1)], 0),
    ("Erddrache", &[(E, 4), (L, 3)], 0),
    ("Elfenbogen", &[(E, 2), (L, 1)], 0),
    ("Drachenzaum", &[(E, 1), (L, 1), (C, 1), (D, 1)], 0),
    ("Flammenpeitsche", &[(E, 2), (D, 2)], 0),
    ("Athanor", &[(G, 1), (E, 1)], 0),
    ("Füllhorn", &[(G, 2)], 0),
    ("Wachhund", &[(E, 1)], 0),
    ("Magische Scherbe", &[], 0),
    ("Prisma", &[], 0),
    ("Opferdolch", &[(D, 1), (G, 1)], 0),
    ("Wasserdrache", &[(C, 6)], 0),
    ("Feuerdrache", &[(E, 6)], 0),
    ("Baum des Lebens", &[(L, 1)], 2),
    ("Falke", &[(L, 1), (C, 1)], 0),
    ("Flammenkelch", &[(G, 1), (E, 1)], 0),
    ("Kelch des Lebens", &[(G, 1), (L, 1), (C, 1)], 0),
    ("Himmlisches Pferd", &[(C, 2), (E, 1)], 0),
    ("Stein der Weisen", &[(E, 2), (L, 2), (C, 2), (D, 2)], 0),
    ("Baumhirte", &[(L, 3), (E, 2)], 0),
    ("Nachtigall", &[(L, 1), (C, 1)], 0),
    ("Jungbrunnen", &[(C, 1), (D, 1)], 0),
    ("Drachenei", &[(G, 1)], 0),
    ("Tresor", &[(G, 1)], 1),
    ("Flammengrube", &[(E, 2)], 0),
    ("Aufziehmann", &[(E, 1), (L, 1), (C, 1), (G, 1)], 0),
    ("Krypta", &[(D, 2)], 3),
    ("Hypnotisches Becken", &[(C, 2), (E, 1), (D, 1)], 0),
    ("Verfluchter Schädel", &[(D, 2)], 0),
    ("Winddrache", &[(C, 4)], 4),
    ("Tanzendes Schwert", &[(G, 1), (E, 1)], 0),
    ("Midas Ring", &[(L, 1), (G, 1)], 0),
];

#[test]
fn every_artifact_is_placed_for_exactly_its_printed_cost() {
    for (name, specific, any) in ARTIFACT_COSTS {
        // Any-essence units are paid with Elan here.
        let mut cost = specific.to_vec();
        cost.push((E, any));
        let table = |pool: &[Qty]| Table::actions().hand(P0, &[name]).pool(P0, pool).start();
        let mut e = table(&cost);
        assert!(can_place(&e, P0, name), "{name} can be placed for {cost:?}");
        place(&mut e, P0, name, &cost);
        assert!(has(&e, P0, TABLEAU, name), "{name} is in the tableau");
        assert_pool(&e, P0, &[]);
        assert!(
            !is_turned(&e, card(&e, P0, name)),
            "{name} begins play straightened"
        );
        assert_eq!(e.current_player(), Some(P1), "placing {name} is an action");
        // One essence less is not enough, unless it is free.
        let paid: u32 = amounts(&cost).values().sum();
        if paid > 0 {
            for (res, _) in amounts(&cost) {
                let mut short = amounts(&cost);
                *short.get_mut(&res).unwrap() -= 1;
                let short: Vec<Qty> = short.into_iter().collect();
                assert!(
                    !can_place(&table(&short), P0, name),
                    "{name} is not free: {short:?}"
                );
            }
        }
    }
}

// (name, printed victory points): all 73 base cards.
const ALL_CARDS: [(&str, i64); 73] = [
    // mages
    ("Heiler", 0),
    ("Verwandlerin", 0),
    ("Gelehrter", 0),
    ("Duellant", 0),
    ("Druidin", 0),
    ("Feuerwerker", 0),
    ("Seherin", 0),
    ("Nekromantin", 0),
    ("Hexe", 0),
    ("Alchemist", 0),
    // monuments
    ("Orakel", 2),
    ("Obelisk", 1),
    ("Salomos Schatzkammer", 1),
    ("Mausoleum", 2),
    ("Hängende Gärten", 1),
    ("Bibliothek", 1),
    ("Große Pyramide", 3),
    ("Goldene Statue", 1),
    ("Tempel", 2),
    ("Koloss", 2),
    // places of power (the points on them count separately)
    ("Verfluchte Schmiede", 1),
    ("Heiliger Hain", 2),
    ("Drachenhort", 0),
    ("Korallenschloss", 3),
    ("Katakomben der Toten", 0),
    // magic items
    ("Forschung", 0),
    ("Umwandlung", 0),
    ("Tod / Leben", 0),
    ("Wahrsagen", 0),
    ("Ruhe / Elan", 0),
    ("Alchemie", 0),
    ("Schutz", 0),
    ("Wiederbelebung", 0),
    // artifacts
    ("Knochendrache", 1),
    ("Seeschlange", 1),
    ("Juwelenstatuette", 1),
    ("Verdorbener Altar", 0),
    ("Meerjungfrau", 0),
    ("Hand des Ruhmes", 0),
    ("Drachenzahn", 0),
    ("Elementarquelle", 0),
    ("Zwergen-Spitzhacke", 0),
    ("Erddrache", 1),
    ("Elfenbogen", 0),
    ("Drachenzaum", 1),
    ("Flammenpeitsche", 0),
    ("Athanor", 0),
    ("Füllhorn", 0),
    ("Wachhund", 0),
    ("Magische Scherbe", 0),
    ("Prisma", 0),
    ("Opferdolch", 0),
    ("Wasserdrache", 1),
    ("Feuerdrache", 1),
    ("Baum des Lebens", 0),
    ("Falke", 0),
    ("Flammenkelch", 0),
    ("Kelch des Lebens", 0),
    ("Himmlisches Pferd", 0),
    ("Stein der Weisen", 1),
    ("Baumhirte", 0),
    ("Nachtigall", 1),
    ("Jungbrunnen", 0),
    ("Drachenei", 1),
    ("Tresor", 0),
    ("Flammengrube", 0),
    ("Aufziehmann", 0),
    ("Krypta", 0),
    ("Hypnotisches Becken", 0),
    ("Verfluchter Schädel", 0),
    ("Winddrache", 1),
    ("Tanzendes Schwert", 0),
    ("Midas Ring", 1),
];

#[test]
fn the_tests_cover_all_73_base_cards_exactly_once() {
    let mut printed: Vec<String> = load().unwrap().into_iter().map(|c| c.name).collect();
    let mut covered: Vec<String> = ALL_CARDS.iter().map(|(name, _)| (*name).to_owned()).collect();
    printed.sort();
    covered.sort();
    assert_eq!(
        covered, printed,
        "a card was added to or removed from the card file"
    );
    assert_eq!(covered.len(), 73);
    let mut costed: Vec<String> = ARTIFACT_COSTS.iter().map(|(n, ..)| (*n).to_owned()).collect();
    let mut artifacts: Vec<String> = load()
        .unwrap()
        .into_iter()
        .filter(|c| c.kind == CardKind::Artifact)
        .map(|c| c.name)
        .collect();
    costed.sort();
    artifacts.sort();
    assert_eq!(
        costed, artifacts,
        "ARTIFACT_COSTS lists every artifact once"
    );
    let count = |kind| load().unwrap().iter().filter(|c| c.kind == kind).count();
    assert_eq!(
        [
            CardKind::Mage,
            CardKind::Monument,
            CardKind::Place,
            CardKind::Item,
            CardKind::Artifact
        ]
        .map(count),
        [10, 10, 5, 8, 40]
    );
}

#[test]
fn every_card_scores_its_printed_victory_points_in_the_tableau() {
    for (name, points) in ALL_CARDS {
        let e = Table::actions().card(P0, name).start();
        assert_eq!(score(&e, P0), points, "{name}");
        assert_eq!(score(&e, P1), 0);
    }
}

#[test]
fn places_of_power_score_for_the_essences_on_them() {
    // (place, points of its own, essence counted, points for each)
    let places = [
        ("Verfluchte Schmiede", 1, G),
        ("Heiliger Hain", 2, L),
        ("Drachenhort", 0, G),
        ("Korallenschloss", 3, E),
        ("Katakomben der Toten", 0, D),
    ];
    for (name, base, counted) in places {
        let e = Table::actions().loaded(P0, name, &[(counted, 3), (C, 2)]).start();
        let per_essence = i64::from(name != "Korallenschloss");
        assert_eq!(
            score(&e, P0),
            base + 3 * per_essence,
            "{name}: only {counted:?} counts"
        );
        // Turned places score as well.
        let e = Table::actions().turned(P0, name).start();
        assert_eq!(score(&e, P0), base, "{name} while turned");
    }
}

// ---------------------------------------------------------------------------------------------------
// More than two players, and the phases around the actions
// ---------------------------------------------------------------------------------------------------

#[test]
fn every_rival_is_hit_or_helped_with_three_players() {
    // The Hand of Glory gives Death to both rivals.
    let mut e = Table::actions().players(3).card(P0, "Hand des Ruhmes").start();
    let hand = card(&e, P0, "Hand des Ruhmes");
    use_power(&mut e, P0, hand, 0);
    assert_eq!(
        (pool(&e, P0, D), pool(&e, P1, D), pool(&e, P2, D)),
        (2, 1, 1)
    );
    // The bow makes both lose a Life; the one with the Healer is asked.
    let mut e = Table::actions()
        .players(3)
        .card(P0, "Elfenbogen")
        .card(P2, "Heiler")
        .pool(P1, &[(L, 2)])
        .pool(P2, &[(L, 2)])
        .start();
    let bow = card(&e, P0, "Elfenbogen");
    use_power(&mut e, P0, bow, 0);
    assert_eq!(
        e.pending().unwrap().players,
        [P2],
        "player 1 has no reaction and just loses"
    );
    assert_eq!(pool(&e, P1, L), 1);
    let healer = card(&e, P2, "Heiler");
    react_with_player(&mut e, P2, healer);
    assert_eq!(pool(&e, P2, L), 2);
}

#[test]
fn treant_and_basin_gain_from_a_rival_of_the_players_choice() {
    let mut e = Table::actions()
        .players(3)
        .card(P0, "Baumhirte")
        .card(P0, "Hypnotisches Becken")
        .pool(P1, &[(D, 2), (E, 1)])
        .pool(P2, &[(D, 3), (E, 4)])
        .start();
    let treant = card(&e, P0, "Baumhirte");
    use_power(&mut e, P0, treant, 0);
    assert_eq!(options(&e).len(), 2, "one option per rival with Death");
    decide_where(&mut e, P0, |o| {
        matches!(
            o,
            Effect::Gain {
                res: ELAN,
                n: 3,
                ..
            }
        )
    });
    assert_pool(&e, P0, &[(E, 3)]);
    // The rest of the round is the same for the other card.
    rivals_pass_in_three(&mut e);
    let basin = card(&e, P0, "Hypnotisches Becken");
    use_power(&mut e, P0, basin, 0);
    assert_eq!(options(&e).len(), 2);
    decide_where(&mut e, P0, |o| {
        matches!(
            o,
            Effect::Gain {
                res: CALM,
                n: 4,
                ..
            }
        )
    });
    assert_pool(&e, P0, &[(E, 3), (C, 4)]);
}

/// Players 1 and 2 pass (and pick magic items), so player 0 acts again.
fn rivals_pass_in_three(e: &mut Game) {
    pass(e, P1);
    pass(e, P2);
    assert_eq!(e.current_player(), Some(P0));
}

#[test]
fn both_players_collect_at_the_start_of_the_round() {
    let e = Table::round().card(P0, "Duellant").card(P1, "Druidin").card(P1, "Nekromantin").start();
    assert!(e.pending().is_none());
    assert_pool(&e, P0, &[(E, 1)]);
    assert_pool(&e, P1, &[(L, 1), (D, 1)]);
}

#[test]
fn the_cleanup_straightens_every_turned_component_of_every_player() {
    let mut e = Table::round()
        .turned(P0, "Elfenbogen")
        .turned(P1, "Wachhund")
        .turned(P1, "Zwergen-Spitzhacke")
        .start();
    assert!(is_turned(&e, card(&e, P0, "Elfenbogen")));
    pass(&mut e, P0);
    pass(&mut e, P1);
    assert_eq!(e.state().flow.round, 2);
    for (p, name) in [
        (P0, "Elfenbogen"),
        (P1, "Wachhund"),
        (P1, "Zwergen-Spitzhacke"),
    ] {
        assert!(!is_turned(&e, card(&e, p, name)), "{name} was straightened");
    }
}

#[test]
fn the_sea_serpent_counts_as_both_a_dragon_and_a_creature() {
    // Dragon: the Lair takes 3 off its 6 Calm, and its second power can turn
    // it.
    let mut e = Table::actions()
        .card(P0, "Drachenhort")
        .hand(P0, &["Seeschlange"])
        .pool(P0, &[(C, 3), (L, 3)])
        .start();
    place(&mut e, P0, "Seeschlange", &[(C, 3), (L, 3)]);
    let e = Table::actions().card(P0, "Drachenhort").card(P0, "Seeschlange").start();
    let lair = card(&e, P0, "Drachenhort");
    assert_eq!(
        power_payments(&e, P0, lair, 1).len(),
        1,
        "the Serpent is a Dragon"
    );
    // Creature: the Druid straightens it and the Grove can turn it.
    let mut e = Table::actions().card(P0, "Druidin").turned(P0, "Seeschlange").start();
    let (druid, serpent) = (card(&e, P0, "Druidin"), card(&e, P0, "Seeschlange"));
    use_power(&mut e, P0, druid, 0);
    assert!(!is_turned(&e, serpent));
    let e = Table::actions().card(P0, "Heiliger Hain").card(P0, "Seeschlange").start();
    let grove = card(&e, P0, "Heiliger Hain");
    assert_eq!(
        power_payments(&e, P0, grove, 1).len(),
        1,
        "the Serpent is a Creature"
    );
}

// END OF THE TESTS

// ---------------------------------------------------------------------------------------------------
// The collect step: the player chooses the order
// ---------------------------------------------------------------------------------------------------

/// The options of the pending collect step, as `(component, is taking
/// essences)`.
fn collect_steps(e: &Game) -> Vec<(EntityId, bool)> {
    options(e)
        .iter()
        .filter_map(|o| {
            collect_option(e, o, false)
                .map(|id| (id, false))
                .or_else(|| collect_option(e, o, true).map(|id| (id, true)))
        })
        .collect()
}

#[test]
fn the_cursed_forge_can_be_paid_with_death_collected_just_before() {
    // No Death in the pool: only the Necromancer's collect ability brings it.
    let table = || Table::round().card(P0, "Verfluchte Schmiede").card(P0, "Nekromantin");
    let raw = |table: Table| Engine::new(table, 2, 1).expect("the table sets up");

    // The Necromancer first: the Death she collects pays the curse.
    let mut e = raw(table());
    let forge = card(&e, P0, "Verfluchte Schmiede");
    let necromancer = card(&e, P0, "Nekromantin");
    assert_eq!(
        collect_steps(&e),
        [(forge, false), (necromancer, false)],
        "both collect abilities are on offer, in the player's order"
    );
    assert!(
        !e.legal_actions(P0).contains(&Action::Decide(0)),
        "the step cannot be ended while a collect ability is owed"
    );
    decide_where(&mut e, P0, |o| {
        collect_option_source(o) == Some(necromancer)
    });
    assert_pool(&e, P0, &[(D, 1)]);
    decide_where(&mut e, P0, |o| collect_option_source(o) == Some(forge));
    decide_where(&mut e, P0, |o| matches!(o, Effect::Pay { .. }));
    assert_pool(&e, P0, &[]);
    assert!(!is_turned(&e, forge), "the curse was paid");
    assert_eq!(e.state().flow.phase_id(), Some(ACTIONS));

    // The Forge first: nothing to pay with, so it turns.
    let mut e = raw(table());
    decide_where(&mut e, P0, |o| collect_option_source(o) == Some(forge));
    assert_eq!(
        options(&e),
        [Effect::Exhaust(Target::Source)],
        "paying is not on offer"
    );
    decide(&mut e, P0, 0);
    assert!(is_turned(&e, forge));
    decide_where(&mut e, P0, |o| {
        collect_option_source(o) == Some(necromancer)
    });
    assert_pool(&e, P0, &[(D, 1)]);
}

#[test]
fn essences_on_components_are_taken_in_the_collect_step_in_any_order() {
    // The Death lying on the Hand of Glory pays the curse when taken first.
    let mut e = Engine::new(
        Table::round().card(P0, "Verfluchte Schmiede").loaded(P0, "Hand des Ruhmes", &[(D, 1)]),
        2,
        1,
    )
    .expect("the table sets up");
    let forge = card(&e, P0, "Verfluchte Schmiede");
    let hand = card(&e, P0, "Hand des Ruhmes");
    assert_eq!(collect_steps(&e), [(forge, false), (hand, true)]);
    decide_where(&mut e, P0, |o| collect_option_source(o) == Some(hand));
    assert_pool(&e, P0, &[(D, 1)]);
    assert_eq!(collect_steps(&e), [(forge, false)], "each step once");
    decide_where(&mut e, P0, |o| collect_option_source(o) == Some(forge));
    decide_where(&mut e, P0, |o| matches!(o, Effect::Pay { .. }));
    assert!(!is_turned(&e, forge));
    assert!(e.pending().is_none(), "nothing left: the actions begin");
    assert_eq!(e.current_player(), Some(P0));

    // Next round the steps are there again.
    pass_raw(&mut e, P0);
    pass_raw(&mut e, P1);
    assert_eq!(e.state().flow.round, 2);
    assert_eq!(collect_steps(&e), [(forge, false)]);
}

/// Passes and picks the first magic item, without touching the collect step
/// of the next round.
fn pass_raw(e: &mut Game, p: PlayerId) {
    e.apply(p, Action::Pass).expect("passing is always possible");
    while picks_an_item(e) {
        decide(e, p, 0);
    }
}
