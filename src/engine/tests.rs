use super::*;
use crate::{
    card::CardDef,
    cost::Cost,
    counter::Counter,
    effect::{Ability, Condition, OwnerRef, Target},
    event::EventKind,
    flow::{EndCheck, Flow, GameEndPolicy, Phase, RoundEndPolicy, TurnOrder, TurnOrderPolicy},
    id::{DefId, PhaseId, ResourceId, ZoneId, ZoneKind},
    market::{Market, MarketPolicy, Refill},
    modifier::Duration,
    zone::{Order, Visibility},
};

const COIN: ResourceId = ResourceId(0);
const GEM: ResourceId = ResourceId(1);
const VP: CounterId = CounterId(0);
const DECK: ZoneKind = ZoneKind(0);
const HAND: ZoneKind = ZoneKind(1);
const DISCARD: ZoneKind = ZoneKind(2);
const TABLEAU: ZoneKind = ZoneKind(3);
const ROW: ZoneKind = ZoneKind(10);
const SUPPLY: ZoneKind = ZoneKind(11);
const P0: PlayerId = PlayerId(0);
const P1: PlayerId = PlayerId(1);
const P2: PlayerId = PlayerId(2);

/// A configurable table: per player deck/hand/discard/tableau (abilities
/// active), a shared market row, VP counters.
#[derive(Clone)]
struct TestRules {
    extra:            fn(&mut GameState, &[PlayerId]),
    phases:           Vec<Phase>,
    round_end:        RoundEndPolicy,
    order:            TurnOrderPolicy,
    actions_per_turn: Option<u32>,
    game_end:         Option<GameEndPolicy>,
    end_effects:      Vec<Effect>,
}

impl TestRules {
    fn new(extra: fn(&mut GameState, &[PlayerId])) -> Self {
        Self {
            extra,
            phases: vec![Phase {
                id:          PhaseId(0),
                interactive: true,
            }],
            round_end: RoundEndPolicy::AllPassed,
            order: TurnOrderPolicy::Clockwise,
            actions_per_turn: None,
            game_end: None,
            end_effects: vec![],
        }
    }
}

impl GameRules for TestRules {
    fn setup(&self, players: &[PlayerId], rng: Rng) -> GameState {
        let mut s = GameState::new(rng);
        s.bank.set_infinite(COIN);
        s.bank.set_infinite(GEM);
        for &p in players {
            s.add_player(p);
            let o = Owner::Player(p);
            let deck = s.add_zone(DECK, o.clone(), Order::Stack, Visibility::Hidden);
            s.add_zone(HAND, o.clone(), Order::Unordered, Visibility::Owner);
            s.add_zone(DISCARD, o.clone(), Order::Stack, Visibility::Public);
            let tableau = s.add_zone(TABLEAU, o.clone(), Order::Unordered, Visibility::Public);
            s.zone_mut(tableau).unwrap().abilities_active = true;
            s.zone_mut(deck).unwrap().recycle_from = Some(DISCARD);
            s.add_counter(Counter::new(VP, o, 0, 0, 100));
        }
        let row = s.add_zone(ROW, Owner::Shared, Order::Row, Visibility::Public);
        let supply = s.add_zone(SUPPLY, Owner::Shared, Order::Stack, Visibility::Hidden);
        s.markets.push(Market {
            zone:   row,
            source: Some(supply),
            size:   3,
            dest:   DISCARD,
            policy: MarketPolicy {
                top_only:       false,
                refill:         Refill::Immediate,
                removal:        None,
                source_buyable: false,
            },
        });
        s.flow = Flow {
            phases: self.phases.clone(),
            turn_order: TurnOrder::new(players.to_vec(), self.order),
            round_end: self.round_end,
            actions_per_turn: self.actions_per_turn,
            game_end: self.game_end.clone(),
            ..Flow::default()
        };
        (self.extra)(&mut s, players);
        s
    }

    fn turn_end_effects(&self, _state: &GameState, _player: PlayerId) -> Vec<Effect> {
        self.end_effects.clone()
    }
}

fn card(id: u32, edit: impl FnOnce(&mut CardDef)) -> CardDef {
    let mut def = CardDef {
        id:        DefId(id),
        name:      format!("card{id}"),
        tags:      vec![],
        cost:      None,
        on_gain:   None,
        abilities: vec![],
        discounts: vec![],
    };
    edit(&mut def);
    def
}

fn activated(cost: Option<Cost>, effect: Effect, limit: Limit) -> Ability {
    Ability {
        trigger: Trigger::Activated,
        condition: Condition::Always,
        cost,
        effect,
        limit,
    }
}

fn gain(res: ResourceId, n: u32) -> Effect {
    Effect::Gain {
        to: Target::Controller,
        res,
        n,
    }
}

fn vp(delta: i64) -> Effect {
    Effect::ModifyCounter {
        counter: VP,
        owner: OwnerRef::Controller,
        delta,
    }
}

fn zone(s: &GameState, owner: &Owner, kind: ZoneKind) -> ZoneId {
    s.zone_id(owner, kind).unwrap()
}

fn put(s: &mut GameState, def: u32, owner: &Owner, kind: ZoneKind, face_up: bool) -> EntityId {
    let z = zone(s, owner, kind);
    s.create_entity(DefId(def), z, face_up).unwrap()
}

fn nothing(_: &mut GameState, _: &[PlayerId]) {}

fn engine(rules: TestRules, players: u8) -> Engine<TestRules> {
    Engine::new(rules, players, 1).unwrap()
}

fn coins<R: GameRules>(e: &Engine<R>, p: PlayerId) -> Option<u32> {
    e.state().player_pool(p).unwrap().finite(COIN)
}

fn vp_of<R: GameRules>(e: &Engine<R>, p: PlayerId) -> i64 {
    e.state().counter(VP, &Owner::Player(p)).unwrap().value
}

fn buys(e: &Engine<TestRules>, p: PlayerId) -> Vec<Action> {
    e.legal_actions(p).into_iter().filter(|a| matches!(a, Action::Buy { .. })).collect()
}

fn activations(e: &Engine<TestRules>, p: PlayerId) -> Vec<Action> {
    e.legal_actions(p).into_iter().filter(|a| matches!(a, Action::Activate { .. })).collect()
}

fn end_turn(e: &mut Engine<TestRules>) {
    let p = e.current_player().unwrap();
    e.apply(p, Action::EndTurn).unwrap();
}

/// A free-to-hold card in the market that costs `cost` coins.
fn market_card(s: &mut GameState, def: CardDef) -> EntityId {
    let id = def.id.0;
    s.add_def(def);
    let row = zone(s, &Owner::Shared, ROW);
    let e = s.create_entity(DefId(id), row, true).unwrap();
    s.markets[0].source = None;
    e
}

// ---- flow --------------------------------------------------------------------------------------------------

#[test]
fn the_game_starts_with_round_one_and_the_first_players_turn() {
    let e = engine(TestRules::new(nothing), 2);
    assert_eq!(e.current_player(), Some(P0));
    let kinds: Vec<_> = e.events().iter().map(Event::kind).collect();
    assert_eq!(
        kinds,
        [
            EventKind::RoundStarted,
            EventKind::PhaseStarted,
            EventKind::TurnStarted
        ]
    );
    assert_eq!(e.events()[0], Event::RoundStarted(1));
    assert_eq!(e.legal_actions(P0), vec![Action::EndTurn, Action::Pass]);
    assert_eq!(e.legal_actions(P1).len(), 0);
}

#[test]
fn ending_a_turn_hands_over_clockwise() {
    let mut e = engine(TestRules::new(nothing), 3);
    end_turn(&mut e);
    assert_eq!(e.current_player(), Some(P1));
    end_turn(&mut e);
    end_turn(&mut e);
    assert_eq!(e.current_player(), Some(P0));
    assert_eq!(e.log().len(), 3);
}

#[test]
fn illegal_actions_are_refused_and_change_nothing() {
    let mut e = engine(TestRules::new(nothing), 2);
    let before = e.state().clone();
    assert_eq!(e.apply(P1, Action::EndTurn), Err(RuleError::Illegal));
    assert_eq!(
        e.apply(
            P0,
            Action::Buy {
                entity: EntityId(9),
                pay:    0,
            }
        ),
        Err(RuleError::Illegal)
    );
    assert_eq!(e.apply(P0, Action::Decide(0)), Err(RuleError::Illegal));
    assert_eq!(e.state(), &before);
    assert_eq!(e.log().len(), 0);
}

#[test]
fn all_passed_ends_the_round_and_pass_order_sets_the_next_order() {
    let mut rules = TestRules::new(nothing);
    rules.order = TurnOrderPolicy::PassOrder;
    let mut e = engine(rules, 3);
    end_turn(&mut e); // P0
    e.apply(P1, Action::Pass).unwrap();
    e.apply(P2, Action::Pass).unwrap();
    assert_eq!(e.current_player(), Some(P0), "passed players are skipped");
    e.apply(P0, Action::Pass).unwrap();
    assert_eq!(e.state().flow.turn_order.order(), &[P1, P2, P0]);
    assert_eq!(e.current_player(), Some(P1));
    assert_eq!(e.state().flow.round, 2);
    assert_eq!(e.state().flow.turn_order.passed().len(), 0);
    assert!(e.events().iter().any(|ev| ev.kind() == EventKind::RoundEnded));
    assert_eq!(
        e.events().iter().filter(|ev| matches!(ev, Event::RoundStarted(_))).count(),
        2
    );
}

#[test]
fn first_pass_ends_the_round_at_once() {
    let mut rules = TestRules::new(nothing);
    rules.round_end = RoundEndPolicy::FirstPass;
    let mut e = engine(rules, 3);
    end_turn(&mut e);
    e.apply(P1, Action::Pass).unwrap();
    assert_eq!(e.state().flow.round, 2);
    assert_eq!(e.current_player(), Some(P0));
}

#[test]
fn non_interactive_phases_resolve_automatically_and_fire_triggers() {
    fn setup(s: &mut GameState, _: &[PlayerId]) {
        // "Collect": on the start of phase 0, the owner gains a coin.
        s.add_def(card(1, |d| {
            d.abilities = vec![Ability {
                trigger:   Trigger::OnEvent(EventKind::PhaseStarted),
                condition: Condition::InPhase(PhaseId(0)),
                cost:      None,
                effect:    gain(COIN, 1),
                limit:     Limit::None,
            }];
        }));
        put(s, 1, &Owner::Player(P0), TABLEAU, true);
    }
    let mut rules = TestRules::new(setup);
    rules.phases = vec![
        Phase {
            id:          PhaseId(0),
            interactive: false,
        },
        Phase {
            id:          PhaseId(1),
            interactive: true,
        },
    ];
    let mut e = engine(rules, 2);
    assert_eq!(
        coins(&e, P0),
        Some(1),
        "collect phase ran before the first turn"
    );
    assert_eq!(e.current_player(), Some(P0));
    // Both pass: the round ends, the collect phase runs again.
    e.apply(P0, Action::Pass).unwrap();
    e.apply(P1, Action::Pass).unwrap();
    assert_eq!(coins(&e, P0), Some(2));
    assert_eq!(e.state().flow.round, 2);
}

#[test]
fn actions_per_turn_ends_the_turn_automatically() {
    fn setup(s: &mut GameState, _: &[PlayerId]) {
        s.add_def(card(1, |d| {
            d.abilities = vec![activated(None, gain(COIN, 1), Limit::None)];
        }));
        put(s, 1, &Owner::Player(P0), TABLEAU, true);
    }
    let mut rules = TestRules::new(setup);
    rules.actions_per_turn = Some(1);
    let mut e = engine(rules, 2);
    let act = activations(&e, P0)[0].clone();
    e.apply(P0, act).unwrap();
    assert_eq!(
        coins(&e, P0),
        Some(1),
        "the effect resolved before the turn ended"
    );
    assert_eq!(e.current_player(), Some(P1));
}

#[test]
fn turn_end_effects_run_for_the_player_whose_turn_ended() {
    fn setup(s: &mut GameState, _: &[PlayerId]) {
        s.add_def(card(1, |_| {}));
        for _ in 0..3 {
            put(s, 1, &Owner::Player(P0), DECK, false);
        }
    }
    let mut rules = TestRules::new(setup);
    rules.end_effects = vec![Effect::Draw {
        who:  OwnerRef::Controller,
        from: DECK,
        to:   HAND,
        n:    2,
    }];
    let mut e = engine(rules, 2);
    end_turn(&mut e);
    let hand = zone(e.state(), &Owner::Player(P0), HAND);
    assert_eq!(e.state().zone(hand).unwrap().len(), 2);
    assert_eq!(e.current_player(), Some(P1));
}

// ---- markets, costs
// ----------------------------------------------------------------------------------------

#[test]
fn buying_pays_moves_the_card_and_resolves_on_gain() {
    fn setup(s: &mut GameState, _: &[PlayerId]) {
        s.bank.set_infinite(COIN);
        s.gain_player(P0, COIN, 3).unwrap();
        market_card(
            s,
            card(1, |d| {
                d.cost = Some(Cost::Resources(vec![(COIN, 2)]));
                d.on_gain = Some(vp(1));
            }),
        );
    }
    let mut e = engine(TestRules::new(setup), 2);
    let buy = buys(&e, P0);
    assert_eq!(buy.len(), 1);
    assert!(buys(&e, P1).is_empty(), "only the active player buys");
    let events = e.apply(P0, buy[0].clone()).unwrap();
    assert_eq!(coins(&e, P0), Some(1));
    assert_eq!(vp_of(&e, P0), 1);
    let discard = zone(e.state(), &Owner::Player(P0), DISCARD);
    assert_eq!(e.state().zone(discard).unwrap().len(), 1);
    assert!(
        events.iter().any(|ev| matches!(ev, Event::CardBought { player, .. } if *player == P0))
    );
    assert!(buys(&e, P0).is_empty(), "the market card is gone");
}

#[test]
fn unaffordable_cards_are_not_offered() {
    fn setup(s: &mut GameState, _: &[PlayerId]) {
        s.gain_player(P0, COIN, 1).unwrap();
        market_card(
            s,
            card(1, |d| d.cost = Some(Cost::Resources(vec![(COIN, 2)]))),
        );
    }
    let e = engine(TestRules::new(setup), 2);
    assert_eq!(buys(&e, P0).len(), 0);
}

#[test]
fn or_costs_offer_one_action_per_payable_alternative() {
    fn setup(s: &mut GameState, _: &[PlayerId]) {
        s.gain_player(P0, COIN, 1).unwrap();
        s.gain_player(P0, GEM, 1).unwrap();
        market_card(
            s,
            card(1, |d| {
                d.cost = Some(Cost::Or(vec![
                    Cost::Resources(vec![(COIN, 3)]),
                    Cost::Resources(vec![(GEM, 1)]),
                ]));
            }),
        );
    }
    let mut e = engine(TestRules::new(setup), 2);
    let buy = buys(&e, P0);
    assert_eq!(buy.len(), 1);
    // `pay` indexes the payments the player can afford: only the Gem
    // alternative is affordable.
    assert!(matches!(buy[0], Action::Buy { pay: 0, .. }));
    e.apply(P0, buy[0].clone()).unwrap();
    let pool = e.state().player_pool(P0).unwrap();
    assert_eq!((pool.finite(COIN), pool.finite(GEM)), (Some(1), Some(0)));
}

#[test]
fn top_only_markets_offer_just_the_top_card() {
    fn setup(s: &mut GameState, _: &[PlayerId]) {
        s.markets[0].policy.top_only = true;
        market_card(s, card(1, |_| {}));
        let row = zone(s, &Owner::Shared, ROW);
        s.create_entity(DefId(1), row, true).unwrap();
    }
    let e = engine(TestRules::new(setup), 2);
    assert_eq!(buys(&e, P0).len(), 1);
}

#[test]
fn end_of_turn_refill_happens_when_the_turn_ends() {
    fn setup(s: &mut GameState, _: &[PlayerId]) {
        s.add_def(card(1, |_| {}));
        s.markets[0].policy.refill = Refill::EndOfTurn;
        let supply = zone(s, &Owner::Shared, SUPPLY);
        let row = zone(s, &Owner::Shared, ROW);
        for _ in 0..2 {
            s.create_entity(DefId(1), supply, false).unwrap();
        }
        s.create_entity(DefId(1), row, true).unwrap();
    }
    let mut e = engine(TestRules::new(setup), 2);
    let row = zone(e.state(), &Owner::Shared, ROW);
    assert_eq!(e.state().zone(row).unwrap().len(), 1);
    end_turn(&mut e);
    assert_eq!(e.state().zone(row).unwrap().len(), 3);
}

// ---- abilities
// ---------------------------------------------------------------------------------------------

#[test]
fn exhaust_cost_makes_an_ability_single_use_until_readied() {
    fn setup(s: &mut GameState, _: &[PlayerId]) {
        s.add_def(card(1, |d| {
            d.abilities = vec![activated(Some(Cost::Exhaust), gain(COIN, 2), Limit::None)];
        }));
        put(s, 1, &Owner::Player(P0), TABLEAU, true);
    }
    let mut e = engine(TestRules::new(setup), 2);
    let act = activations(&e, P0);
    assert_eq!(act.len(), 1);
    e.apply(P0, act[0].clone()).unwrap();
    assert_eq!(coins(&e, P0), Some(2));
    assert_eq!(activations(&e, P0).len(), 0);
    let tableau = zone(e.state(), &Owner::Player(P0), TABLEAU);
    let card = e.state().zone(tableau).unwrap().top().unwrap();
    assert!(e.state().entity(card).unwrap().exhausted);
}

#[test]
fn once_per_turn_resets_at_the_end_of_the_turn() {
    fn setup(s: &mut GameState, _: &[PlayerId]) {
        s.add_def(card(1, |d| {
            d.abilities = vec![activated(None, gain(COIN, 1), Limit::OncePerTurn)];
        }));
        put(s, 1, &Owner::Player(P0), TABLEAU, true);
    }
    let mut e = engine(TestRules::new(setup), 2);
    e.apply(P0, activations(&e, P0)[0].clone()).unwrap();
    assert_eq!(activations(&e, P0).len(), 0);
    end_turn(&mut e);
    end_turn(&mut e);
    assert_eq!(activations(&e, P0).len(), 1);
}

#[test]
fn once_between_turns_works_on_other_players_turns_too() {
    fn setup(s: &mut GameState, _: &[PlayerId]) {
        s.add_def(card(1, |d| {
            d.abilities = vec![Ability {
                trigger:   Trigger::OnEvent(EventKind::TurnEnded),
                condition: Condition::OtherTurn,
                cost:      None,
                effect:    gain(COIN, 1),
                limit:     Limit::OncePerBetweenTurns,
            }];
        }));
        put(s, 1, &Owner::Player(P0), TABLEAU, true);
    }
    let mut e = engine(TestRules::new(setup), 3);
    end_turn(&mut e); // P0's own turn: condition fails
    assert_eq!(coins(&e, P0), Some(0));
    end_turn(&mut e); // P1's turn ends: fires
    end_turn(&mut e); // P2's turn ends: already used between P0's turns
    assert_eq!(coins(&e, P0), Some(1));
    end_turn(&mut e); // P0's turn started again -> reset, now P0's turn ends (own turn, no fire)
    end_turn(&mut e); // P1 ends again
    assert_eq!(coins(&e, P0), Some(2));
}

#[test]
fn triggered_abilities_fire_in_turn_order_starting_with_the_active_player() {
    fn setup(s: &mut GameState, _: &[PlayerId]) {
        // Each player's card: when anyone's turn starts, the owner gains a coin
        // and the owner's VP rises.
        s.add_def(card(1, |d| {
            d.abilities = vec![Ability {
                trigger:   Trigger::OnEvent(EventKind::TurnEnded),
                condition: Condition::Always,
                cost:      None,
                effect:    Effect::Sequence(vec![gain(COIN, 1), vp(1)]),
                limit:     Limit::None,
            }];
        }));
        for p in [P0, P1] {
            put(s, 1, &Owner::Player(p), TABLEAU, true);
        }
    }
    let mut e = engine(TestRules::new(setup), 2);
    end_turn(&mut e);
    let order: Vec<_> = e
        .events()
        .iter()
        .filter_map(|ev| match ev {
            Event::ResourceGained {
                to: Subject::Player(p),
                ..
            } => Some(*p),
            _ => None,
        })
        .collect();
    assert_eq!(
        order,
        [P0, P1],
        "the active player's trigger resolves first"
    );
    end_turn(&mut e);
    let order: Vec<_> = e
        .events()
        .iter()
        .filter_map(|ev| match ev {
            Event::ResourceGained {
                to: Subject::Player(p),
                ..
            } => Some(*p),
            _ => None,
        })
        .collect();
    assert_eq!(&order[2..], [P1, P0]);
    assert_eq!((vp_of(&e, P0), vp_of(&e, P1)), (2, 2));
}

#[test]
fn resources_can_be_gained_onto_the_card_itself() {
    fn setup(s: &mut GameState, _: &[PlayerId]) {
        s.add_def(card(1, |d| {
            d.abilities = vec![activated(
                None,
                Effect::Gain {
                    to:  Target::Source,
                    res: GEM,
                    n:   2,
                },
                Limit::None,
            )];
        }));
        put(s, 1, &Owner::Player(P0), TABLEAU, true);
    }
    let mut e = engine(TestRules::new(setup), 2);
    e.apply(P0, activations(&e, P0)[0].clone()).unwrap();
    let tableau = zone(e.state(), &Owner::Player(P0), TABLEAU);
    let card = e.state().zone(tableau).unwrap().top().unwrap();
    assert_eq!(e.state().entity(card).unwrap().pool.finite(GEM), Some(2));
    assert_eq!(e.state().player_pool(P0).unwrap().finite(GEM), Some(0));
}

#[test]
fn a_move_effect_relocates_the_source_card() {
    fn setup(s: &mut GameState, _: &[PlayerId]) {
        s.add_def(card(1, |d| {
            d.abilities = vec![activated(
                None,
                Effect::Move {
                    entity: Target::Source,
                    to:     DISCARD,
                    owner:  OwnerRef::Controller,
                },
                Limit::None,
            )];
        }));
        put(s, 1, &Owner::Player(P0), TABLEAU, true);
    }
    let mut e = engine(TestRules::new(setup), 2);
    e.apply(P0, activations(&e, P0)[0].clone()).unwrap();
    let discard = zone(e.state(), &Owner::Player(P0), DISCARD);
    assert_eq!(e.state().zone(discard).unwrap().len(), 1);
    assert!(
        activations(&e, P0).is_empty(),
        "off the tableau the ability is inactive"
    );
}

#[test]
fn team_zones_let_every_member_activate_but_nobody_else() {
    fn setup(s: &mut GameState, _: &[PlayerId]) {
        let team = Owner::Players(vec![P0, P1]);
        let z = s.add_zone(ZoneKind(20), team, Order::Unordered, Visibility::Public);
        s.zone_mut(z).unwrap().abilities_active = true;
        s.add_def(card(1, |d| {
            d.abilities = vec![activated(None, vp(1), Limit::None)];
        }));
        s.create_entity(DefId(1), z, true).unwrap();
    }
    let mut e = engine(TestRules::new(setup), 3);
    assert_eq!(activations(&e, P0).len(), 1);
    end_turn(&mut e);
    assert_eq!(
        activations(&e, P1).len(),
        1,
        "a team member may use it on their turn"
    );
    end_turn(&mut e);
    assert!(activations(&e, P2).is_empty(), "a non-member may not");
}

#[test]
fn team_zones_are_visible_to_members_only_when_owner_visibility() {
    fn setup(s: &mut GameState, _: &[PlayerId]) {
        s.add_def(card(1, |_| {}));
        let z = s.add_zone(
            ZoneKind(20),
            Owner::Players(vec![P0, P1]),
            Order::Unordered,
            Visibility::Owner,
        );
        s.create_entity(DefId(1), z, false).unwrap();
    }
    let e = engine(TestRules::new(setup), 3);
    let seen = |p| {
        let v = e.view(p);
        let z = v.zones.iter().find(|z| z.kind == ZoneKind(20)).unwrap().clone();
        (
            z.count,
            z.entities.len(),
            z.entities.first().and_then(|x| x.def),
        )
    };
    assert_eq!(seen(P1), (1, 1, Some(DefId(1))));
    assert_eq!(seen(P2), (1, 0, None));
}

// ---- decisions
// ---------------------------------------------------------------------------------------------

#[test]
fn choose_pauses_the_engine_until_the_player_decides() {
    fn setup(s: &mut GameState, _: &[PlayerId]) {
        s.add_def(card(1, |d| {
            d.abilities = vec![activated(
                None,
                Effect::Choose(vec![gain(COIN, 1), vp(1)]),
                Limit::None,
            )];
        }));
        put(s, 1, &Owner::Player(P0), TABLEAU, true);
    }
    let mut e = engine(TestRules::new(setup), 2);
    e.apply(P0, activations(&e, P0)[0].clone()).unwrap();
    assert!(e.pending().is_some());
    assert_eq!(
        e.legal_actions(P0),
        vec![Action::Decide(0), Action::Decide(1)]
    );
    assert_eq!(e.legal_actions(P1).len(), 0);
    e.apply(P0, Action::Decide(1)).unwrap();
    assert!(e.pending().is_none());
    assert_eq!((vp_of(&e, P0), coins(&e, P0)), (1, Some(0)));
    assert!(e.legal_actions(P0).contains(&Action::EndTurn));
}

#[test]
fn a_decision_in_the_middle_of_a_sequence_resumes_the_rest() {
    fn setup(s: &mut GameState, _: &[PlayerId]) {
        s.add_def(card(1, |d| {
            let seq = Effect::Sequence(vec![
                gain(COIN, 1),
                Effect::Choose(vec![vp(1), vp(2)]),
                gain(COIN, 10),
            ]);
            d.abilities = vec![activated(None, seq, Limit::None)];
        }));
        put(s, 1, &Owner::Player(P0), TABLEAU, true);
    }
    let mut e = engine(TestRules::new(setup), 2);
    e.apply(P0, activations(&e, P0)[0].clone()).unwrap();
    assert_eq!(coins(&e, P0), Some(1), "stopped at the choice");
    e.apply(P0, Action::Decide(1)).unwrap();
    assert_eq!((coins(&e, P0), vp_of(&e, P0)), (Some(11), 2));
}

#[test]
fn simultaneous_decisions_wait_for_everyone_and_stay_secret() {
    fn setup(s: &mut GameState, _: &[PlayerId]) {
        s.pending = Some(PendingDecision {
            players: vec![P0, P1],
            options: vec![gain(COIN, 1), gain(COIN, 5)],
            answers: vec![],
            source:  None,
            bound:   crate::effect::Bound::default(),
        });
    }
    let mut e = engine(TestRules::new(setup), 2);
    assert!(
        e.current_player().is_none(),
        "the game waits for the draft before starting"
    );
    assert_eq!(e.legal_actions(P0).len(), 2);
    assert_eq!(e.view(P1).pending.unwrap().options.unwrap().len(), 2);
    e.apply(P1, Action::Decide(1)).unwrap();
    assert!(e.legal_actions(P1).is_empty(), "P1 already answered");
    assert_eq!(e.view(P1).pending.unwrap().options, None);
    assert_eq!(e.view(P0).pending.unwrap().waiting_on, vec![P0]);
    assert_eq!(
        coins(&e, P1),
        Some(0),
        "nothing resolves until everyone answered"
    );
    e.apply(P0, Action::Decide(0)).unwrap();
    assert_eq!((coins(&e, P0), coins(&e, P1)), (Some(1), Some(5)));
    assert_eq!(e.current_player(), Some(P0), "now the game starts");
}

// ---- modifiers
// ---------------------------------------------------------------------------------------------

#[test]
fn a_curse_forbids_buying_on_the_victims_next_turn_only() {
    fn setup(s: &mut GameState, _: &[PlayerId]) {
        market_card(s, card(1, |_| {}));
        market_card(s, card(2, |_| {}));
        market_card(s, card(3, |_| {}));
        let curse = Effect::AddModifier {
            target:   Target::Player(P1),
            rule:     RuleChange::Forbid(ActionKind::Buy),
            duration: Duration::NextTurnOf(P1),
        };
        s.add_def(card(9, |d| {
            d.abilities = vec![activated(None, curse, Limit::None)];
        }));
        put(s, 9, &Owner::Player(P0), TABLEAU, true);
    }
    let mut e = engine(TestRules::new(setup), 2);
    e.apply(P0, activations(&e, P0)[0].clone()).unwrap();
    assert_eq!(e.state().modifiers.len(), 1);
    assert!(
        !buys(&e, P0).is_empty(),
        "the curse does not touch the caster"
    );
    end_turn(&mut e);
    assert!(buys(&e, P1).is_empty(), "cursed during their next turn");
    end_turn(&mut e);
    assert!(
        e.state().modifiers.is_empty(),
        "the curse ended with that turn"
    );
    end_turn(&mut e);
    assert!(!buys(&e, P1).is_empty(), "free again on the turn after");
}

#[test]
fn cost_modifiers_raise_the_price_for_their_target() {
    fn setup(s: &mut GameState, _: &[PlayerId]) {
        s.gain_player(P0, COIN, 2).unwrap();
        market_card(
            s,
            card(1, |d| d.cost = Some(Cost::Resources(vec![(COIN, 2)]))),
        );
        let tax = Effect::AddModifier {
            target:   Target::Controller,
            rule:     RuleChange::CostDelta {
                res:   COIN,
                delta: 1,
            },
            duration: Duration::ThisTurn,
        };
        s.add_def(card(9, |d| {
            d.abilities = vec![activated(None, tax, Limit::None)];
        }));
        put(s, 9, &Owner::Player(P0), TABLEAU, true);
    }
    let mut e = engine(TestRules::new(setup), 2);
    assert_eq!(buys(&e, P0).len(), 1);
    e.apply(P0, activations(&e, P0)[0].clone()).unwrap();
    assert!(
        buys(&e, P0).is_empty(),
        "2 coins no longer pay a 3-coin price"
    );
}

#[test]
fn forbidding_activation_blocks_a_card() {
    fn setup(s: &mut GameState, _: &[PlayerId]) {
        s.add_def(card(1, |d| {
            d.abilities = vec![activated(None, vp(1), Limit::None)];
        }));
        let e = put(s, 1, &Owner::Player(P0), TABLEAU, true);
        s.modifiers.push(Modifier::new(
            Subject::Entity(e),
            RuleChange::Forbid(ActionKind::Activate),
            Duration::Permanent,
        ));
    }
    let e = engine(TestRules::new(setup), 2);
    assert_eq!(activations(&e, P0).len(), 0);
}

// ---- game end
// ----------------------------------------------------------------------------------------------

fn vp_policy(when: EndCheck, target: i64) -> GameEndPolicy {
    GameEndPolicy {
        counter: VP,
        target,
        when,
        tiebreak: vec![],
    }
}

#[test]
fn reaching_the_target_after_an_event_ends_the_game_at_once() {
    fn setup(s: &mut GameState, _: &[PlayerId]) {
        s.add_def(card(1, |d| {
            d.abilities = vec![activated(None, vp(3), Limit::None)];
        }));
        put(s, 1, &Owner::Player(P0), TABLEAU, true);
    }
    let mut rules = TestRules::new(setup);
    rules.game_end = Some(vp_policy(EndCheck::AfterEvent, 3));
    let mut e = engine(rules, 2);
    let events = e.apply(P0, activations(&e, P0)[0].clone()).unwrap();
    assert_eq!(e.winners(), Some([P0].as_slice()));
    assert!(matches!(events.last(), Some(Event::GameEnded(w)) if w == &vec![P0]));
    assert_eq!(e.legal_actions(P0).len() + e.legal_actions(P1).len(), 0);
    assert_eq!(e.apply(P0, Action::EndTurn), Err(RuleError::GameOver));
}

#[test]
fn a_round_end_check_waits_for_the_round_to_finish() {
    fn setup(s: &mut GameState, _: &[PlayerId]) {
        s.add_def(card(1, |d| {
            d.abilities = vec![activated(None, vp(2), Limit::None)];
        }));
        put(s, 1, &Owner::Player(P1), TABLEAU, true);
    }
    let mut rules = TestRules::new(setup);
    rules.game_end = Some(vp_policy(EndCheck::RoundEnd, 2));
    let mut e = engine(rules, 2);
    end_turn(&mut e);
    e.apply(P1, activations(&e, P1)[0].clone()).unwrap();
    assert_eq!(vp_of(&e, P1), 2);
    assert!(
        e.winners().is_none(),
        "target reached mid-round, nobody wins yet"
    );
    e.apply(P1, Action::Pass).unwrap();
    e.apply(P0, Action::Pass).unwrap();
    assert_eq!(e.winners(), Some([P1].as_slice()));
}

// ---- views, replay, safety
// ---------------------------------------------------------------------------------

#[test]
fn views_hide_what_the_viewer_may_not_see() {
    fn setup(s: &mut GameState, _: &[PlayerId]) {
        s.add_def(card(1, |_| {}));
        s.add_def(card(2, |_| {}));
        put(s, 1, &Owner::Player(P0), HAND, false);
        put(s, 1, &Owner::Player(P0), DECK, false);
        put(s, 2, &Owner::Player(P0), TABLEAU, true);
        put(s, 1, &Owner::Player(P0), TABLEAU, false);
    }
    let e = engine(TestRules::new(setup), 2);
    let find = |v: &PlayerView, kind| {
        v.zones.iter().find(|z| z.owner == Owner::Player(P0) && z.kind == kind).unwrap().clone()
    };
    let (mine, theirs) = (e.view(P0), e.view(P1));

    let hand = find(&mine, HAND);
    assert_eq!(
        (hand.count, hand.entities.len(), hand.entities[0].def),
        (1, 1, Some(DefId(1)))
    );
    let hand = find(&theirs, HAND);
    assert_eq!((hand.count, hand.entities.len()), (1, 0));

    for v in [&mine, &theirs] {
        let deck = find(v, DECK);
        assert_eq!(
            (deck.count, deck.entities.len()),
            (1, 0),
            "decks are hidden even from their owner"
        );
    }

    let tableau = find(&theirs, TABLEAU);
    assert_eq!(tableau.entities.len(), 2);
    assert_eq!(
        tableau.entities[0].def,
        Some(DefId(2)),
        "face-up card in a public zone"
    );
    assert_eq!(
        tableau.entities[1].def, None,
        "face-down card in a public zone"
    );
    assert_eq!(
        find(&mine, TABLEAU).entities[1].def,
        Some(DefId(1)),
        "the owner knows their own cards"
    );
}

#[test]
fn replaying_the_log_reproduces_the_exact_state() {
    fn setup(s: &mut GameState, _: &[PlayerId]) {
        s.add_def(card(1, |_| {}));
        s.add_def(card(2, |d| {
            d.abilities = vec![activated(None, gain(COIN, 1), Limit::OncePerTurn)];
        }));
        for p in [P0, P1] {
            for _ in 0..6 {
                put(s, 1, &Owner::Player(p), DECK, false);
            }
            put(s, 2, &Owner::Player(p), TABLEAU, true);
            let deck = zone(s, &Owner::Player(p), DECK);
            s.shuffle(deck).unwrap();
        }
    }
    let mut rules = TestRules::new(setup);
    rules.end_effects = vec![
        Effect::MoveAll {
            from:  HAND,
            to:    DISCARD,
            owner: OwnerRef::Controller,
        },
        Effect::Draw {
            who:  OwnerRef::Controller,
            from: DECK,
            to:   HAND,
            n:    2,
        },
    ];
    let mut e = Engine::new(rules.clone(), 2, 77).unwrap();
    let mut pick = Rng::from_seed(5);
    for _ in 0..40 {
        let p = e.current_player().unwrap();
        let legal = e.legal_actions(p);
        let action = legal[pick.below(legal.len())].clone();
        e.apply(p, action).unwrap();
    }
    let replayed = Engine::replay(rules, 2, 77, e.log()).unwrap();
    assert_eq!(replayed.state(), e.state());
    assert_eq!(replayed.events(), e.events());
    let json = serde_json::to_string(e.state()).unwrap();
    assert_eq!(
        &serde_json::from_str::<GameState>(&json).unwrap(),
        e.state()
    );
}

#[test]
fn a_different_seed_gives_a_different_game() {
    fn setup(s: &mut GameState, _: &[PlayerId]) {
        s.add_def(card(1, |_| {}));
        for _ in 0..20 {
            put(s, 1, &Owner::Player(P0), DECK, false);
        }
        let deck = zone(s, &Owner::Player(P0), DECK);
        s.shuffle(deck).unwrap();
    }
    let order = |seed| {
        let e = Engine::new(TestRules::new(setup), 2, seed).unwrap();
        let deck = zone(e.state(), &Owner::Player(P0), DECK);
        e.state().zone(deck).unwrap().entities().to_vec()
    };
    assert_eq!(order(1), order(1));
    assert_ne!(order(1), order(2));
}

#[test]
fn an_ability_that_triggers_itself_forever_is_stopped_and_rolled_back() {
    fn setup(s: &mut GameState, _: &[PlayerId]) {
        s.add_def(card(1, |d| {
            d.abilities = vec![
                activated(None, gain(COIN, 1), Limit::None),
                Ability {
                    trigger:   Trigger::OnEvent(EventKind::ResourceGained),
                    condition: Condition::Always,
                    cost:      None,
                    effect:    gain(COIN, 1),
                    limit:     Limit::None,
                },
            ];
        }));
        put(s, 1, &Owner::Player(P0), TABLEAU, true);
    }
    let mut e = engine(TestRules::new(setup), 2);
    let before = e.state().clone();
    let events_before = e.events().len();
    let act = activations(&e, P0)[0].clone();
    assert_eq!(e.apply(P0, act), Err(RuleError::Runaway));
    assert_eq!(e.state(), &before);
    assert_eq!(e.events().len(), events_before);
    assert_eq!(e.log().len(), 0);
}

#[test]
fn rules_errors_surface_and_roll_back() {
    fn setup(s: &mut GameState, _: &[PlayerId]) {
        s.add_def(card(1, |d| {
            d.abilities = vec![activated(
                None,
                Effect::Sequence(vec![gain(COIN, 1), vp(1)]),
                Limit::None,
            )];
        }));
        put(s, 1, &Owner::Player(P0), TABLEAU, true);
        s.counters.clear();
    }
    let mut e = engine(TestRules::new(setup), 2);
    let before = e.state().clone();
    let act = activations(&e, P0)[0].clone();
    assert_eq!(
        e.apply(P0, act),
        Err(RuleError::NoCounter(VP, Owner::Player(P0)))
    );
    assert_eq!(
        e.state(),
        &before,
        "the coin gained before the failure was rolled back"
    );
}

#[test]
fn a_game_without_phases_is_reported() {
    let mut rules = TestRules::new(nothing);
    rules.phases.clear();
    assert!(matches!(Engine::new(rules, 2, 1), Err(RuleError::NoPhases)));
}

// ---- extension: effects
// -------------------------------------------------------------------------
//
// One test per effect the engine resolves for the extension. Cards live in the
// tableau of player 0; each has one activated ability without a cost unless the
// test is about the cost.

use crate::{
    cost::{Count, Which},
    effect::Filter,
    id::EntityId as Eid,
    resource::Supply,
    state::CardRules,
};

fn ra_table(s: &mut GameState) {
    s.card_rules = Some(CardRules {
        hand:       HAND,
        tableau:    TABLEAU,
        discard:    DISCARD,
        on_discard: None,
    });
}

/// The first entity created from a definition.
fn find(e: &Engine<TestRules>, def: u32) -> Eid {
    e.state().entities().find(|x| x.def == DefId(def)).map(|x| x.id).unwrap()
}

/// Uses ability `ability` of `entity` with the first affordable payment.
fn use_ability(e: &mut Engine<TestRules>, p: PlayerId, entity: Eid, ability: usize) {
    let action = e
        .legal_actions(p)
        .into_iter()
        .find(|a| matches!(a, Action::Activate { entity: x, ability: i, .. } if *x == entity && *i == ability))
        .expect("the ability is legal");
    e.apply(p, action).unwrap();
}

/// Uses ability `ability` of the first card made from definition `def`.
fn use_def(e: &mut Engine<TestRules>, p: PlayerId, def: u32, ability: usize) {
    let entity = find(e, def);
    use_ability(e, p, entity, ability);
}

fn on_card(e: &Engine<TestRules>, entity: Eid, res: ResourceId) -> Supply {
    e.state().entity(entity).unwrap().pool.get(res)
}

fn single(effect: Effect) -> Ability {
    activated(None, effect, Limit::None)
}

#[test]
fn pay_takes_from_the_pool_and_a_short_pool_rolls_everything_back() {
    let rules = TestRules::new(|s, _| {
        ra_table(s);
        s.add_def(card(1, |d| {
            d.abilities = vec![single(Effect::Pay { res: COIN, n: 2 })];
        }));
        s.add_def(card(2, |d| {
            d.abilities = vec![single(Effect::Pay { res: COIN, n: 9 })];
        }));
        put(s, 1, &Owner::Player(P0), TABLEAU, true);
        put(s, 2, &Owner::Player(P0), TABLEAU, true);
        s.gain_player(P0, COIN, 3).unwrap();
    });
    let mut e = engine(rules, 2);
    let before = e.state().clone();
    let too_much = find(&e, 2);
    let action = e
        .legal_actions(P0)
        .into_iter()
        .find(|a| matches!(a, Action::Activate { entity, .. } if *entity == too_much))
        .unwrap();
    assert!(e.apply(P0, action).is_err());
    assert_eq!(e.state(), &before);
    use_def(&mut e, P0, 1, 0);
    assert_eq!(coins(&e, P0), Some(1));
}

#[test]
fn exhaust_ready_and_ready_all_change_the_turned_state() {
    let rules = TestRules::new(|s, _| {
        ra_table(s);
        s.add_def(card(1, |d| {
            d.abilities = vec![
                single(Effect::Exhaust(Target::Source)),
                single(Effect::Ready(Target::Source)),
                single(Effect::ReadyAll {
                    owner: OwnerRef::Controller,
                }),
            ];
        }));
        s.add_def(card(2, |_| {}));
        put(s, 1, &Owner::Player(P0), TABLEAU, true);
        let other = put(s, 2, &Owner::Player(P0), TABLEAU, true);
        s.entity_mut(other).unwrap().exhausted = true;
    });
    let mut e = engine(rules, 2);
    let (a, b) = (find(&e, 1), find(&e, 2));
    use_ability(&mut e, P0, a, 0);
    assert!(e.state().entity(a).unwrap().exhausted);
    use_ability(&mut e, P0, a, 1);
    assert!(!e.state().entity(a).unwrap().exhausted);
    use_ability(&mut e, P0, a, 0);
    use_ability(&mut e, P0, a, 2);
    assert!(!e.state().entity(a).unwrap().exhausted);
    assert!(!e.state().entity(b).unwrap().exhausted);
}

#[test]
fn take_all_moves_everything_from_a_card_to_the_controller() {
    let rules = TestRules::new(|s, _| {
        ra_table(s);
        s.add_def(card(1, |d| {
            d.abilities = vec![single(Effect::TakeAll {
                from: Target::Source,
            })];
        }));
        let c = put(s, 1, &Owner::Player(P0), TABLEAU, true);
        s.gain_entity(c, COIN, 2).unwrap();
        s.gain_entity(c, GEM, 1).unwrap();
    });
    let mut e = engine(rules, 2);
    let c = find(&e, 1);
    use_ability(&mut e, P0, c, 0);
    assert_eq!(coins(&e, P0), Some(2));
    assert_eq!(e.state().player_pool(P0).unwrap().finite(GEM), Some(1));
    assert_eq!(on_card(&e, c, COIN), Supply::Finite(0));
    assert_eq!(on_card(&e, c, GEM), Supply::Finite(0));
}

#[test]
fn grow_adds_to_every_kind_already_on_the_card() {
    let rules = TestRules::new(|s, _| {
        ra_table(s);
        s.add_def(card(1, |d| {
            d.abilities = vec![single(Effect::Grow { per_kind: 1 })];
        }));
        let c = put(s, 1, &Owner::Player(P0), TABLEAU, true);
        s.gain_entity(c, COIN, 1).unwrap();
        s.gain_entity(c, GEM, 2).unwrap();
    });
    let mut e = engine(rules, 2);
    let c = find(&e, 1);
    use_ability(&mut e, P0, c, 0);
    assert_eq!(on_card(&e, c, COIN), Supply::Finite(2));
    assert_eq!(on_card(&e, c, GEM), Supply::Finite(3));
}

#[test]
fn destroy_empties_the_card_and_discards_it_to_its_owner() {
    let rules = TestRules::new(|s, _| {
        ra_table(s);
        s.add_def(card(1, |d| {
            d.abilities = vec![single(Effect::Destroy {
                target: Target::Source,
            })];
        }));
        let c = put(s, 1, &Owner::Player(P1), TABLEAU, true);
        s.gain_entity(c, COIN, 2).unwrap();
        // The ability sits on P1's card but P1 is not the active player: give
        // P0 a copy to use.
        s.add_def(card(2, |d| {
            d.abilities = vec![single(Effect::Destroy {
                target: Target::Source,
            })];
        }));
        let d = put(s, 2, &Owner::Player(P0), TABLEAU, true);
        s.gain_entity(d, COIN, 2).unwrap();
    });
    let mut e = engine(rules, 2);
    let d = find(&e, 2);
    use_ability(&mut e, P0, d, 0);
    assert_eq!(on_card(&e, d, COIN), Supply::Finite(0));
    let discard = e.state().zone_id(&Owner::Player(P0), DISCARD).unwrap();
    assert_eq!(e.state().zone(discard).unwrap().entities(), [d]);
    // The other player's card is untouched.
    let c = find(&e, 1);
    assert_eq!(on_card(&e, c, COIN), Supply::Finite(2));
}

#[test]
fn a_variable_cost_binds_x_for_gain_x() {
    let rules = TestRules::new(|s, _| {
        ra_table(s);
        s.add_def(card(1, |d| {
            d.abilities = vec![activated(
                Some(Cost::Any {
                    n:         Count::Var,
                    exclude:   vec![],
                    identical: true,
                }),
                Effect::GainX {
                    to:  Target::Controller,
                    res: GEM,
                },
                Limit::None,
            )];
        }));
        put(s, 1, &Owner::Player(P0), TABLEAU, true);
        s.gain_player(P0, COIN, 3).unwrap();
    });
    let mut e = engine(rules, 2);
    let c = find(&e, 1);
    let all = payments(e.state(), P0, Some(c), activations_cost(&e, c).as_ref(), c);
    let two = all.iter().position(|p| p.x == Some(2)).unwrap();
    e.apply(
        P0,
        Action::Activate {
            entity:  c,
            ability: 0,
            pay:     two,
        },
    )
    .unwrap();
    assert_eq!(coins(&e, P0), Some(1));
    assert_eq!(e.state().player_pool(P0).unwrap().finite(GEM), Some(2));
}

/// The cost of ability 0 of a card.
fn activations_cost(e: &Engine<TestRules>, entity: Eid) -> Option<Cost> {
    abilities_of(e.state(), entity)[0].cost.clone()
}

#[test]
fn gain_paid_puts_the_single_paid_resource_on_a_card() {
    let rules = TestRules::new(|s, _| {
        ra_table(s);
        s.add_def(card(1, |d| {
            d.abilities = vec![activated(
                Some(Cost::Any {
                    n:         Count::Fixed(1),
                    exclude:   vec![],
                    identical: false,
                }),
                Effect::GainPaid { to: Target::Source },
                Limit::None,
            )];
        }));
        put(s, 1, &Owner::Player(P0), TABLEAU, true);
        s.gain_player(P0, COIN, 1).unwrap();
        s.gain_player(P0, GEM, 1).unwrap();
    });
    let mut e = engine(rules, 2);
    let c = find(&e, 1);
    let all = payments(e.state(), P0, Some(c), activations_cost(&e, c).as_ref(), c);
    let gem = all.iter().position(|p| p.single_paid() == Some(GEM)).unwrap();
    e.apply(
        P0,
        Action::Activate {
            entity:  c,
            ability: 0,
            pay:     gem,
        },
    )
    .unwrap();
    assert_eq!(on_card(&e, c, GEM), Supply::Finite(1));
    assert_eq!(on_card(&e, c, COIN), Supply::Finite(0));
    assert_eq!(e.state().player_pool(P0).unwrap().finite(GEM), Some(0));
}

#[test]
fn a_destroyed_or_discarded_cost_card_is_bound_to_target_card() {
    let rules = TestRules::new(|s, _| {
        ra_table(s);
        s.add_def(card(1, |d| {
            d.abilities = vec![activated(
                Some(Cost::Destroy {
                    which: Which::Other,
                    tag:   "fodder".into(),
                }),
                Effect::Gain {
                    to:  Target::Card,
                    res: COIN,
                    n:   1,
                },
                Limit::None,
            )];
        }));
        s.add_def(card(2, |d| d.tags = vec!["fodder".into()]));
        s.add_def(card(3, |d| {
            d.abilities = vec![activated(
                Some(Cost::Discard {
                    from: HAND,
                    n:    1,
                }),
                Effect::Gain {
                    to:  Target::Card,
                    res: GEM,
                    n:   1,
                },
                Limit::None,
            )];
        }));
        s.add_def(card(4, |_| {}));
        put(s, 1, &Owner::Player(P0), TABLEAU, true);
        put(s, 2, &Owner::Player(P0), TABLEAU, true);
        put(s, 3, &Owner::Player(P0), TABLEAU, true);
        put(s, 4, &Owner::Player(P0), HAND, true);
    });
    let mut e = engine(rules, 2);
    let (user, fodder, discarder, held) = (find(&e, 1), find(&e, 2), find(&e, 3), find(&e, 4));
    use_ability(&mut e, P0, user, 0);
    assert_eq!(on_card(&e, fodder, COIN), Supply::Finite(1));
    assert_ne!(
        e.state().entity(fodder).unwrap().zone,
        e.state().zone_id(&Owner::Player(P0), TABLEAU).unwrap()
    );
    use_ability(&mut e, P0, discarder, 0);
    assert_eq!(on_card(&e, held, GEM), Supply::Finite(1));
    assert_ne!(
        e.state().entity(held).unwrap().zone,
        e.state().zone_id(&Owner::Player(P0), HAND).unwrap()
    );
}

#[test]
fn gain_all_hits_rivals_or_everyone() {
    let rules = TestRules::new(|s, _| {
        ra_table(s);
        s.add_def(card(1, |d| {
            d.abilities = vec![
                single(Effect::GainAll {
                    who: Who::Rivals,
                    res: COIN,
                    n:   1,
                }),
                single(Effect::GainAll {
                    who: Who::Everyone,
                    res: GEM,
                    n:   2,
                }),
            ];
        }));
        put(s, 1, &Owner::Player(P0), TABLEAU, true);
    });
    let mut e = engine(rules, 3);
    let c = find(&e, 1);
    use_ability(&mut e, P0, c, 0);
    assert_eq!(coins(&e, P0), Some(0));
    assert_eq!(coins(&e, P1), Some(1));
    assert_eq!(coins(&e, P2), Some(1));
    use_ability(&mut e, P0, c, 1);
    for p in [P0, P1, P2] {
        assert_eq!(e.state().player_pool(p).unwrap().finite(GEM), Some(2));
    }
}

#[test]
fn move_nth_moves_the_card_at_that_position() {
    let rules = TestRules::new(|s, _| {
        ra_table(s);
        s.add_def(card(1, |d| {
            d.abilities = vec![single(Effect::MoveNth {
                from:  HAND,
                index: 1,
                to:    DISCARD,
                owner: OwnerRef::Controller,
            })];
        }));
        s.add_def(card(2, |_| {}));
        put(s, 1, &Owner::Player(P0), TABLEAU, true);
        put(s, 2, &Owner::Player(P0), HAND, true);
        put(s, 2, &Owner::Player(P0), HAND, true);
    });
    let mut e = engine(rules, 2);
    let hand = e.state().zone_id(&Owner::Player(P0), HAND).unwrap();
    let second = e.state().zone(hand).unwrap().entities()[1];
    use_def(&mut e, P0, 1, 0);
    assert_eq!(e.state().zone(hand).unwrap().entities().len(), 1);
    assert_eq!(
        e.state().entity(second).unwrap().zone,
        e.state().zone_id(&Owner::Player(P0), DISCARD).unwrap()
    );
}

#[test]
fn reset_counters_and_for_each_player_touch_every_player() {
    let rules = TestRules::new(|s, _| {
        ra_table(s);
        s.add_def(card(1, |d| {
            d.abilities = vec![
                single(Effect::ForEachPlayer(Box::new(vp(2)))),
                single(Effect::ResetCounters { counter: VP }),
            ];
        }));
        put(s, 1, &Owner::Player(P0), TABLEAU, true);
    });
    let mut e = engine(rules, 3);
    let c = find(&e, 1);
    use_ability(&mut e, P0, c, 0);
    for p in [P0, P1, P2] {
        assert_eq!(vp_of(&e, p), 2);
    }
    use_ability(&mut e, P0, c, 1);
    for p in [P0, P1, P2] {
        assert_eq!(vp_of(&e, p), 0);
    }
}

#[test]
fn for_each_owned_applies_to_every_card_passing_the_filter() {
    let rules = TestRules::new(|s, _| {
        ra_table(s);
        s.add_def(card(1, |d| {
            d.abilities = vec![single(Effect::ForEachOwned {
                filter: Filter {
                    tag:           Some("target".into()),
                    exhausted:     Some(false),
                    has_resources: None,
                },
                effect: Box::new(Effect::Exhaust(Target::Card)),
            })];
        }));
        s.add_def(card(2, |d| d.tags = vec!["target".into()]));
        s.add_def(card(3, |_| {}));
        put(s, 1, &Owner::Player(P0), TABLEAU, true);
        put(s, 2, &Owner::Player(P0), TABLEAU, true);
        put(s, 2, &Owner::Player(P0), TABLEAU, true);
        put(s, 3, &Owner::Player(P0), TABLEAU, true);
    });
    let mut e = engine(rules, 2);
    use_def(&mut e, P0, 1, 0);
    for x in e.state().entities() {
        assert_eq!(
            x.exhausted,
            x.def == DefId(2),
            "only the tagged cards are turned"
        );
    }
}

fn pick_rules(candidates: u32) -> TestRules {
    TestRules::new(match candidates {
        0 => |s, _| pick_table(s, 0),
        1 => |s, _| pick_table(s, 1),
        _ => |s, _| pick_table(s, 2),
    })
}

fn pick_table(s: &mut GameState, candidates: u32) {
    ra_table(s);
    s.add_def(card(1, |d| {
        d.abilities = vec![single(Effect::Pick {
            from:   TABLEAU,
            owner:  OwnerRef::Controller,
            filter: Filter {
                tag: Some("target".into()),
                ..Filter::default()
            },
            then:   Box::new(Effect::Exhaust(Target::Card)),
        })];
    }));
    s.add_def(card(2, |d| d.tags = vec!["target".into()]));
    put(s, 1, &Owner::Player(P0), TABLEAU, true);
    for _ in 0..candidates {
        put(s, 2, &Owner::Player(P0), TABLEAU, true);
    }
}

#[test]
fn pick_asks_only_when_there_is_a_real_choice() {
    // No candidate: nothing happens. One: no question. Two: a decision, then
    // the picked card is turned.
    let mut none = engine(pick_rules(0), 2);
    use_def(&mut none, P0, 1, 0);
    assert!(none.pending().is_none());

    let mut one = engine(pick_rules(1), 2);
    use_def(&mut one, P0, 1, 0);
    assert!(one.pending().is_none());
    assert!(one.state().entity(find(&one, 2)).unwrap().exhausted);

    let mut two = engine(pick_rules(2), 2);
    use_def(&mut two, P0, 1, 0);
    assert_eq!(two.pending().unwrap().options.len(), 2);
    two.apply(P0, Action::Decide(1)).unwrap();
    let turned: Vec<_> = two.state().entities().filter(|x| x.exhausted).map(|x| x.id).collect();
    assert_eq!(turned.len(), 1);
    assert_eq!(two.state().entity(turned[0]).unwrap().def, DefId(2));
}

// ---- extension: placing and discarding
// ----------------------------------------------------------

use crate::card::Discount;

fn cost_of(coins: u32, gems: u32) -> Cost {
    let mut parts = vec![];
    if coins > 0 {
        parts.push((COIN, coins));
    }
    if gems > 0 {
        parts.push((GEM, gems));
    }
    Cost::Resources(parts)
}

fn places(e: &Engine<TestRules>, p: PlayerId) -> Vec<Action> {
    e.legal_actions(p).into_iter().filter(|a| matches!(a, Action::Place { .. })).collect()
}

fn zone_of(e: &Engine<TestRules>, entity: Eid) -> ZoneId {
    e.state().entity(entity).unwrap().zone
}

fn owned(e: &Engine<TestRules>, p: PlayerId, kind: ZoneKind) -> ZoneId {
    e.state().zone_id(&Owner::Player(p), kind).unwrap()
}

#[test]
fn placing_pays_the_cost_moves_the_card_and_uses_the_action() {
    let short = TestRules::new(|s, _| {
        ra_table(s);
        s.add_def(card(1, |d| d.cost = Some(cost_of(2, 1))));
        put(s, 1, &Owner::Player(P0), HAND, true);
        s.gain_player(P0, COIN, 3).unwrap();
    });
    // Not enough gems: nothing to place.
    assert_eq!(places(&engine(short, 2), P0).len(), 0);

    let mut rules = TestRules::new(|s, _| {
        ra_table(s);
        s.add_def(card(1, |d| d.cost = Some(cost_of(2, 1))));
        put(s, 1, &Owner::Player(P0), HAND, true);
        s.gain_player(P0, COIN, 3).unwrap();
        s.gain_player(P0, GEM, 1).unwrap();
    });
    rules.actions_per_turn = Some(1);
    let mut e = engine(rules, 2);
    let c = find(&e, 1);
    assert_eq!(
        places(&e, P0),
        [Action::Place {
            entity: c,
            pay:    0,
        }]
    );
    e.apply(
        P0,
        Action::Place {
            entity: c,
            pay:    0,
        },
    )
    .unwrap();
    assert_eq!(zone_of(&e, c), owned(&e, P0, TABLEAU));
    assert_eq!(coins(&e, P0), Some(1));
    assert_eq!(e.state().player_pool(P0).unwrap().finite(GEM), Some(0));
    // One action per turn: the turn moved on.
    assert_eq!(e.current_player(), Some(P1));
}

#[test]
fn discounts_from_cards_in_play_lower_the_price_and_stack() {
    let rules = TestRules::new(|s, _| {
        ra_table(s);
        // A tagged discount (-1 for "art"), a general one (-1), and one that
        // does not apply.
        s.add_def(card(10, |d| {
            d.discounts = vec![Discount {
                tag:     Some("art".into()),
                n:       1,
                exclude: vec![],
            }];
        }));
        s.add_def(card(11, |d| {
            d.discounts = vec![Discount {
                tag:     None,
                n:       1,
                exclude: vec![],
            }];
        }));
        s.add_def(card(12, |d| {
            d.discounts = vec![Discount {
                tag:     Some("other".into()),
                n:       5,
                exclude: vec![],
            }];
        }));
        s.add_def(card(1, |d| {
            d.tags = vec!["art".into()];
            d.cost = Some(cost_of(3, 0));
        }));
        for def in [10, 11, 12] {
            put(s, def, &Owner::Player(P0), TABLEAU, true);
        }
        put(s, 1, &Owner::Player(P0), HAND, true);
        s.gain_player(P0, COIN, 1).unwrap();
    });
    let mut e = engine(rules, 2);
    let c = find(&e, 1);
    assert_eq!(
        places(&e, P0),
        [Action::Place {
            entity: c,
            pay:    0,
        }]
    );
    e.apply(
        P0,
        Action::Place {
            entity: c,
            pay:    0,
        },
    )
    .unwrap();
    assert_eq!(
        coins(&e, P0),
        Some(0),
        "3 coins minus 2 discount = 1 coin paid"
    );
}

#[test]
fn a_stale_or_unknown_place_payment_is_refused() {
    let rules = TestRules::new(|s, _| {
        ra_table(s);
        s.add_def(card(1, |d| d.cost = Some(cost_of(1, 0))));
        put(s, 1, &Owner::Player(P0), HAND, true);
        s.gain_player(P0, COIN, 1).unwrap();
    });
    let mut e = engine(rules, 2);
    let c = find(&e, 1);
    let before = e.state().clone();
    assert_eq!(
        e.apply(
            P0,
            Action::Place {
                entity: c,
                pay:    1,
            }
        ),
        Err(RuleError::Illegal)
    );
    assert_eq!(
        e.apply(
            P1,
            Action::Place {
                entity: c,
                pay:    0,
            }
        ),
        Err(RuleError::Illegal)
    );
    assert_eq!(e.state(), &before);
}

#[test]
fn discarding_moves_the_card_and_resolves_the_reward() {
    let rules = TestRules::new(|s, _| {
        ra_table(s);
        s.card_rules.as_mut().unwrap().on_discard = Some(gain(COIN, 1));
        s.add_def(card(1, |_| {}));
        put(s, 1, &Owner::Player(P0), HAND, true);
        put(s, 1, &Owner::Player(P1), HAND, true);
    });
    let mut e = engine(rules, 2);
    let mine = e.state().zone(owned(&e, P0, HAND)).unwrap().entities()[0];
    let theirs = e.state().zone(owned(&e, P1, HAND)).unwrap().entities()[0];
    // Only your own hand is offered.
    let offered: Vec<_> =
        e.legal_actions(P0).into_iter().filter(|a| matches!(a, Action::Discard { .. })).collect();
    assert_eq!(offered, [Action::Discard { entity: mine }]);
    assert_eq!(
        e.apply(P0, Action::Discard { entity: theirs }),
        Err(RuleError::Illegal)
    );
    e.apply(P0, Action::Discard { entity: mine }).unwrap();
    assert_eq!(zone_of(&e, mine), owned(&e, P0, DISCARD));
    assert_eq!(coins(&e, P0), Some(1));
}

fn place_card_rules() -> TestRules {
    TestRules::new(|s, _| {
        ra_table(s);
        s.add_def(card(1, |d| {
            d.abilities = vec![single(Effect::PlaceCard {
                from:     None,
                tag:      Some("art".into()),
                discount: Some(Discount {
                    tag:     None,
                    n:       2,
                    exclude: vec![],
                }),
            })];
        }));
        s.add_def(card(2, |d| {
            d.tags = vec!["art".into()];
            d.cost = Some(cost_of(3, 0));
        }));
        s.add_def(card(3, |d| d.cost = Some(cost_of(1, 0))));
        put(s, 1, &Owner::Player(P0), TABLEAU, true);
        put(s, 2, &Owner::Player(P0), HAND, true);
        put(s, 3, &Owner::Player(P0), HAND, true);
        s.gain_player(P0, COIN, 1).unwrap();
    })
}

#[test]
fn place_card_offers_matching_cards_with_the_extra_discount_and_may_be_declined() {
    let mut e = engine(place_card_rules(), 2);
    use_def(&mut e, P0, 1, 0);
    // Only the tagged card (3 - 2 = 1 coin) is offered, plus declining.
    let pending = e.pending().unwrap();
    assert_eq!(pending.options.len(), 2);
    assert_eq!(pending.options[1], Effect::Ignore);
    e.apply(P0, Action::Decide(0)).unwrap();
    let placed = find(&e, 2);
    assert_eq!(zone_of(&e, placed), owned(&e, P0, TABLEAU));
    assert_eq!(coins(&e, P0), Some(0));
    assert_eq!(zone_of(&e, find(&e, 3)), owned(&e, P0, HAND));

    let mut e = engine(place_card_rules(), 2);
    use_def(&mut e, P0, 1, 0);
    e.apply(P0, Action::Decide(1)).unwrap();
    assert_eq!(zone_of(&e, find(&e, 2)), owned(&e, P0, HAND));
    assert_eq!(coins(&e, P0), Some(1));
}

#[test]
fn place_card_with_nothing_affordable_does_not_ask() {
    let rules = TestRules::new(|s, _| {
        ra_table(s);
        s.add_def(card(1, |d| {
            d.abilities = vec![single(Effect::PlaceCard {
                from:     None,
                tag:      None,
                discount: None,
            })];
        }));
        s.add_def(card(2, |d| d.cost = Some(cost_of(5, 0))));
        put(s, 1, &Owner::Player(P0), TABLEAU, true);
        put(s, 2, &Owner::Player(P0), HAND, true);
    });
    let mut e = engine(rules, 2);
    use_def(&mut e, P0, 1, 0);
    assert!(e.pending().is_none());
}

// ---- extension: choices
// -------------------------------------------------------------------------


/// Player 0 owns one tableau card (definition 1) whose only ability resolves
/// `effect`.
fn one_card(s: &mut GameState, effect: Effect) {
    ra_table(s);
    s.add_def(card(1, |d| d.abilities = vec![single(effect)]));
    put(s, 1, &Owner::Player(P0), TABLEAU, true);
}

fn gain_choose(n: Count, exclude: Vec<ResourceId>, identical: bool) -> Effect {
    Effect::GainChoose {
        to: Target::Controller,
        n,
        exclude,
        identical,
    }
}

fn gems(e: &Engine<TestRules>, p: PlayerId) -> Option<u32> {
    e.state().player_pool(p).unwrap().finite(GEM)
}

#[test]
fn gain_choose_offers_every_mix_and_gives_the_picked_one() {
    let rules = TestRules::new(|s, _| one_card(s, gain_choose(Count::Fixed(2), vec![], false)));
    let mut e = engine(rules, 2);
    use_def(&mut e, P0, 1, 0);
    // Mixes of 2 over {coin, gem}, in a fixed order: 2 gems, 1 + 1, 2 coins.
    assert_eq!(e.pending().unwrap().options.len(), 3);
    e.apply(P0, Action::Decide(1)).unwrap();
    assert_eq!((coins(&e, P0), gems(&e, P0)), (Some(1), Some(1)));
}

#[test]
fn gain_choose_identical_offers_one_option_per_kind() {
    let rules = TestRules::new(|s, _| one_card(s, gain_choose(Count::Fixed(2), vec![], true)));
    let mut e = engine(rules, 2);
    use_def(&mut e, P0, 1, 0);
    assert_eq!(e.pending().unwrap().options.len(), 2);
    e.apply(P0, Action::Decide(0)).unwrap();
    assert_eq!((coins(&e, P0), gems(&e, P0)), (Some(2), Some(0)));
}

#[test]
fn gain_choose_without_a_real_choice_does_not_ask() {
    let rules = TestRules::new(|s, _| one_card(s, gain_choose(Count::Fixed(2), vec![GEM], false)));
    let mut e = engine(rules, 2);
    use_def(&mut e, P0, 1, 0);
    assert!(e.pending().is_none());
    assert_eq!((coins(&e, P0), gems(&e, P0)), (Some(2), Some(0)));
}

#[test]
fn gain_per_rival_picks_a_rival_and_copies_their_count() {
    let rules = TestRules::new(|s, _| {
        one_card(
            s,
            Effect::GainPerRival {
                res:     COIN,
                counted: GEM,
            },
        );
        s.gain_player(P1, GEM, 2).unwrap();
        s.gain_player(P2, GEM, 3).unwrap();
    });
    let mut e = engine(rules, 3);
    use_def(&mut e, P0, 1, 0);
    assert_eq!(e.pending().unwrap().options.len(), 2);
    e.apply(P0, Action::Decide(1)).unwrap();
    assert_eq!(coins(&e, P0), Some(3));

    // Equal counts are one option; the choice disappears.
    let rules = TestRules::new(|s, _| {
        one_card(
            s,
            Effect::GainPerRival {
                res:     COIN,
                counted: GEM,
            },
        );
        s.gain_player(P1, GEM, 2).unwrap();
        s.gain_player(P2, GEM, 2).unwrap();
    });
    let mut e = engine(rules, 3);
    use_def(&mut e, P0, 1, 0);
    assert!(e.pending().is_none());
    assert_eq!(coins(&e, P0), Some(2));
}

#[test]
fn discard_choose_asks_one_card_at_a_time() {
    let rules = TestRules::new(|s, _| {
        one_card(s, Effect::DiscardChoose { n: 2 });
        s.add_def(card(2, |_| {}));
        for _ in 0..3 {
            put(s, 2, &Owner::Player(P0), HAND, true);
        }
    });
    let mut e = engine(rules, 2);
    use_def(&mut e, P0, 1, 0);
    assert_eq!(e.pending().unwrap().options.len(), 3);
    e.apply(P0, Action::Decide(0)).unwrap();
    assert_eq!(e.pending().unwrap().options.len(), 2);
    e.apply(P0, Action::Decide(1)).unwrap();
    assert!(e.pending().is_none());
    assert_eq!(e.state().zone(owned(&e, P0, HAND)).unwrap().len(), 1);
    assert_eq!(e.state().zone(owned(&e, P0, DISCARD)).unwrap().len(), 2);
}

#[test]
fn discard_choose_with_few_cards_discards_the_whole_hand() {
    let rules = TestRules::new(|s, _| {
        one_card(s, Effect::DiscardChoose { n: 3 });
        s.add_def(card(2, |_| {}));
        for _ in 0..2 {
            put(s, 2, &Owner::Player(P0), HAND, true);
        }
    });
    let mut e = engine(rules, 2);
    use_def(&mut e, P0, 1, 0);
    assert!(e.pending().is_none());
    assert_eq!(e.state().zone(owned(&e, P0, HAND)).unwrap().len(), 0);
    assert_eq!(e.state().zone(owned(&e, P0, DISCARD)).unwrap().len(), 2);
}

fn deck_of(s: &mut GameState, cards: usize, effect: Effect) {
    one_card(s, effect);
    s.add_def(card(2, |_| {}));
    for _ in 0..cards {
        put(s, 2, &Owner::Player(P0), DECK, false);
    }
}

fn reorder_effect() -> Effect {
    Effect::Reorder {
        owner: OwnerRef::Controller,
        zone:  DECK,
        n:     3,
    }
}

#[test]
fn reorder_lets_the_controller_pick_the_order_of_the_top_cards() {
    let rules = TestRules::new(|s, _| deck_of(s, 4, reorder_effect()));
    let mut e = engine(rules, 2);
    let deck = owned(&e, P0, DECK);
    let before = e.state().zone(deck).unwrap().entities().to_vec();
    use_def(&mut e, P0, 1, 0);
    assert_eq!(e.pending().unwrap().options.len(), 6);
    // The last permutation reverses the top three; the card under them stays.
    e.apply(P0, Action::Decide(5)).unwrap();
    let after = e.state().zone(deck).unwrap().entities().to_vec();
    assert_eq!(after, [before[0], before[3], before[2], before[1]]);
}

#[test]
fn reorder_with_fewer_than_two_cards_does_not_ask() {
    let rules = TestRules::new(|s, _| deck_of(s, 1, reorder_effect()));
    let mut e = engine(rules, 2);
    use_def(&mut e, P0, 1, 0);
    assert!(e.pending().is_none());

    // With two cards, "the top 3" means the two there are.
    let rules = TestRules::new(|s, _| deck_of(s, 2, reorder_effect()));
    let mut e = engine(rules, 2);
    use_def(&mut e, P0, 1, 0);
    assert_eq!(e.pending().unwrap().options.len(), 2);
}

#[test]
fn reorder_top_refuses_a_bad_permutation_and_changes_nothing() {
    let rules = TestRules::new(|s, _| deck_of(s, 3, Effect::Ignore));
    let e = engine(rules, 2);
    let deck = owned(&e, P0, DECK);
    let mut s = e.state().clone();
    assert!(s.reorder_top(deck, &[0, 0]).is_err());
    assert!(s.reorder_top(deck, &[0, 1, 2, 3]).is_err());
    assert!(s.reorder_top(deck, &[1, 2]).is_err());
    assert_eq!(&s, e.state());
}

// ---- extension: effects aimed at several players, reactions, win checks
// -------------------------------------
//
// The engine knows nothing about what a reaction is *for*: the test game
// defines three kinds of its own.

use crate::{
    effect::{Defense, Escape},
    id::ReactKind,
};

const WARD: ReactKind = ReactKind(0);
const SPECIAL_WARD: ReactKind = ReactKind(1);
const CHECK: ReactKind = ReactKind(2);
const JEWEL: ResourceId = ResourceId(2);

fn react(kind: ReactKind, cost: Option<Cost>, effect: Effect) -> Ability {
    Ability {
        trigger: Trigger::React(kind),
        condition: Condition::Always,
        cost,
        effect,
        limit: Limit::None,
    }
}

/// "Everyone in `who` loses 2 gems (any 2 other things per missing gem), unless
/// they defend."
fn harm(who: Who, react: Vec<ReactKind>, escape: Option<Escape>, skip_passed: bool) -> Effect {
    Effect::Affect {
        who,
        defense: Defense {
            react,
            escape,
            skip_passed,
        },
        effect: Box::new(Effect::Lose {
            res:       GEM,
            n:         2,
            shortfall: 2,
        }),
    }
}

fn rivals_harmed(s: &mut GameState) {
    one_card(s, harm(Who::Rivals, vec![WARD], None, true));
}

/// A ready card of player `p` with a React ability, definition `def`.
fn warded(s: &mut GameState, def: u32, p: PlayerId, ability: Ability) {
    s.add_def(card(def, |d| d.abilities = vec![ability]));
    put(s, def, &Owner::Player(p), TABLEAU, true);
}

#[test]
fn affect_without_defenses_hits_rivals_in_order_and_shortfalls_cost_other_resources() {
    let rules = TestRules::new(|s, _| {
        rivals_harmed(s);
        s.gain_player(P1, GEM, 3).unwrap();
        s.gain_player(P2, GEM, 1).unwrap();
        s.gain_player(P2, COIN, 3).unwrap();
    });
    let mut e = engine(rules, 3);
    use_def(&mut e, P0, 1, 0);
    assert!(e.pending().is_none());
    assert_eq!(gems(&e, P1), Some(1));
    assert_eq!(gems(&e, P2), Some(0));
    assert_eq!(
        coins(&e, P2),
        Some(1),
        "one gem short: two other resources lost"
    );
    assert_eq!(gems(&e, P0), Some(0), "the controller is not a rival");
}

#[test]
fn a_react_ability_replaces_the_effect_and_the_next_player_waits_for_it() {
    let rules = TestRules::new(|s, _| {
        rivals_harmed(s);
        warded(s, 2, P1, react(WARD, Some(Cost::Exhaust), Effect::Ignore));
        s.gain_player(P1, GEM, 3).unwrap();
        s.gain_player(P2, GEM, 3).unwrap();
    });
    let mut e = engine(rules, 3);
    use_def(&mut e, P0, 1, 0);
    let pending = e.pending().unwrap();
    assert_eq!(pending.players, [P1]);
    assert_eq!(pending.options.len(), 2);
    assert_eq!(
        gems(&e, P2),
        Some(3),
        "player 2 is served after player 1 answered"
    );
    e.apply(P1, Action::Decide(1)).unwrap();
    assert_eq!(gems(&e, P1), Some(3));
    assert!(e.state().entity(find(&e, 2)).unwrap().exhausted);
    assert!(e.pending().is_none());
    assert_eq!(gems(&e, P2), Some(1));
}

#[test]
fn declining_a_reaction_takes_the_effect_and_a_turned_card_cannot_react() {
    let rules = TestRules::new(|s, _| {
        rivals_harmed(s);
        warded(s, 2, P1, react(WARD, Some(Cost::Exhaust), Effect::Ignore));
        s.gain_player(P1, GEM, 3).unwrap();
    });
    let mut e = engine(rules, 2);
    use_def(&mut e, P0, 1, 0);
    e.apply(P1, Action::Decide(0)).unwrap();
    assert_eq!(gems(&e, P1), Some(1));

    let rules = TestRules::new(|s, _| {
        rivals_harmed(s);
        warded(s, 2, P1, react(WARD, Some(Cost::Exhaust), Effect::Ignore));
        let c = find_in(s, 2);
        s.entity_mut(c).unwrap().exhausted = true;
        s.gain_player(P1, GEM, 3).unwrap();
    });
    let mut e = engine(rules, 2);
    use_def(&mut e, P0, 1, 0);
    assert!(
        e.pending().is_none(),
        "no way to react: the effect just happens"
    );
    assert_eq!(gems(&e, P1), Some(1));
}

fn find_in(s: &GameState, def: u32) -> Eid {
    s.entities().find(|x| x.def == DefId(def)).map(|x| x.id).unwrap()
}

#[test]
fn a_reaction_can_cost_resources_and_run_its_own_effect() {
    let rules = TestRules::new(|s, _| {
        rivals_harmed(s);
        warded(
            s,
            2,
            P1,
            react(
                WARD,
                Some(cost_of(1, 0)),
                Effect::Gain {
                    to:  Target::Source,
                    res: JEWEL,
                    n:   1,
                },
            ),
        );
        s.bank.set_infinite(JEWEL);
        s.gain_player(P1, COIN, 1).unwrap();
        s.gain_player(P1, GEM, 3).unwrap();
    });
    let mut e = engine(rules, 2);
    use_def(&mut e, P0, 1, 0);
    e.apply(P1, Action::Decide(1)).unwrap();
    assert_eq!(coins(&e, P1), Some(0), "the cost was paid");
    assert_eq!(gems(&e, P1), Some(3), "the loss did not happen");
    assert_eq!(
        on_card(&e, find(&e, 2), JEWEL),
        Supply::Finite(1),
        "the ability ran with its card as source"
    );
}

#[test]
fn only_the_reaction_kinds_the_effect_names_are_offered() {
    let rules = TestRules::new(|s, _| {
        rivals_harmed(s);
        warded(s, 2, P1, react(SPECIAL_WARD, None, Effect::Ignore));
        s.gain_player(P1, GEM, 3).unwrap();
    });
    let mut e = engine(rules, 2);
    use_def(&mut e, P0, 1, 0);
    assert!(e.pending().is_none());
    assert_eq!(gems(&e, P1), Some(1));

    let rules = TestRules::new(|s, _| {
        one_card(s, harm(Who::Rivals, vec![WARD, SPECIAL_WARD], None, true));
        warded(s, 2, P1, react(SPECIAL_WARD, None, Effect::Ignore));
        s.gain_player(P1, GEM, 3).unwrap();
    });
    let mut e = engine(rules, 2);
    use_def(&mut e, P0, 1, 0);
    assert_eq!(e.pending().unwrap().options.len(), 2);
}

#[test]
fn passed_players_are_skipped_only_when_the_defense_says_so() {
    for (skip, expected) in [(true, 3), (false, 1)] {
        let rules = if skip {
            TestRules::new(|s, _| {
                one_card(s, harm(Who::Rivals, vec![], None, true));
                s.gain_player(P1, GEM, 3).unwrap();
            })
        } else {
            TestRules::new(|s, _| {
                one_card(s, harm(Who::Rivals, vec![], None, false));
                s.gain_player(P1, GEM, 3).unwrap();
            })
        };
        let mut e = engine(rules, 3);
        end_turn(&mut e);
        e.apply(P1, Action::Pass).unwrap();
        end_turn(&mut e);
        assert_eq!(e.current_player(), Some(P0));
        use_def(&mut e, P0, 1, 0);
        assert_eq!(gems(&e, P1), Some(expected), "skip_passed = {skip}");
    }
}

#[test]
fn lose_asks_which_other_resources_and_takes_what_exists_when_short() {
    let rules = TestRules::new(|s, _| {
        rivals_harmed(s);
        s.bank.set_infinite(JEWEL);
        s.gain_player(P1, COIN, 2).unwrap();
        s.gain_player(P1, JEWEL, 2).unwrap();
    });
    let mut e = engine(rules, 2);
    use_def(&mut e, P0, 1, 0);
    // Two gems short, so 4 other resources: all of them, no choice left.
    assert!(e.pending().is_none());
    assert_eq!(
        (
            coins(&e, P1),
            e.state().player_pool(P1).unwrap().finite(JEWEL)
        ),
        (Some(0), Some(0))
    );

    let rules = TestRules::new(|s, _| {
        rivals_harmed(s);
        s.bank.set_infinite(JEWEL);
        s.gain_player(P1, GEM, 1).unwrap();
        s.gain_player(P1, COIN, 2).unwrap();
        s.gain_player(P1, JEWEL, 2).unwrap();
    });
    let mut e = engine(rules, 2);
    use_def(&mut e, P0, 1, 0);
    // One gem short: any 2 of {2 coins, 2 jewels}: 3 mixes.
    assert_eq!(e.pending().unwrap().options.len(), 3);
    e.apply(P1, Action::Decide(1)).unwrap();
    assert_eq!(gems(&e, P1), Some(0));
    assert_eq!(
        (
            coins(&e, P1),
            e.state().player_pool(P1).unwrap().finite(JEWEL)
        ),
        (Some(1), Some(1))
    );
}

#[test]
fn escapes_are_offered_only_when_they_can_be_used() {
    // Pay a coin.
    let paying = |coin: u32| {
        TestRules::new(match coin {
            0 => |s, _| {
                one_card(
                    s,
                    harm(Who::Rivals, vec![], Some(Escape::Pay(COIN, 1)), true),
                );
                s.gain_player(P1, GEM, 3).unwrap();
            },
            _ => |s, _| {
                one_card(
                    s,
                    harm(Who::Rivals, vec![], Some(Escape::Pay(COIN, 1)), true),
                );
                s.gain_player(P1, GEM, 3).unwrap();
                s.gain_player(P1, COIN, 1).unwrap();
            },
        })
    };
    let mut e = engine(paying(0), 2);
    use_def(&mut e, P0, 1, 0);
    assert!(e.pending().is_none(), "cannot pay: nothing to choose");
    assert_eq!(gems(&e, P1), Some(1));
    let mut e = engine(paying(1), 2);
    use_def(&mut e, P0, 1, 0);
    assert_eq!(e.pending().unwrap().options.len(), 2);
    e.apply(P1, Action::Decide(1)).unwrap();
    assert_eq!((gems(&e, P1), coins(&e, P1)), (Some(3), Some(0)));

    // Discard a card.
    let rules = TestRules::new(|s, _| {
        one_card(
            s,
            harm(Who::Rivals, vec![], Some(Escape::DiscardCard), true),
        );
        s.add_def(card(2, |_| {}));
        put(s, 2, &Owner::Player(P1), HAND, true);
        s.gain_player(P1, GEM, 3).unwrap();
    });
    let mut e = engine(rules, 2);
    use_def(&mut e, P0, 1, 0);
    e.apply(P1, Action::Decide(1)).unwrap();
    assert_eq!(gems(&e, P1), Some(3));
    assert_eq!(zone_of(&e, find(&e, 2)), owned(&e, P1, DISCARD));

    // Destroy a tagged card of yours.
    let rules = TestRules::new(|s, _| {
        one_card(
            s,
            harm(
                Who::Rivals,
                vec![],
                Some(Escape::Destroy("fodder".into())),
                true,
            ),
        );
        s.add_def(card(2, |d| d.tags = vec!["fodder".into()]));
        put(s, 2, &Owner::Player(P1), TABLEAU, true);
        s.gain_player(P1, GEM, 3).unwrap();
    });
    let mut e = engine(rules, 2);
    use_def(&mut e, P0, 1, 0);
    e.apply(P1, Action::Decide(1)).unwrap();
    assert_eq!(gems(&e, P1), Some(3));
    assert_eq!(zone_of(&e, find(&e, 2)), owned(&e, P1, DISCARD));
}

#[test]
fn everyone_includes_the_controller_who_is_served_last() {
    let rules = TestRules::new(|s, _| {
        one_card(s, harm(Who::Everyone, vec![], None, true));
        for p in [P0, P1] {
            s.gain_player(p, GEM, 2).unwrap();
        }
    });
    let mut e = engine(rules, 2);
    use_def(&mut e, P0, 1, 0);
    assert_eq!((gems(&e, P0), gems(&e, P1)), (Some(0), Some(0)));
}

/// "Every player may react, then check who has won", as a game would build it.
fn check_window() -> Effect {
    Effect::Sequence(vec![
        Effect::ForEachPlayer(Box::new(Effect::ReactWindow {
            kind:    CHECK,
            default: Box::new(Effect::Ignore),
            repeat:  true,
        })),
        Effect::CheckWin,
    ])
}

fn two_boosts(s: &mut GameState) {
    one_card(s, check_window());
    warded(s, 2, P0, react(CHECK, Some(Cost::Exhaust), vp(1)));
    warded(s, 3, P0, react(CHECK, Some(Cost::Exhaust), vp(1)));
}

fn target_two(rules: &mut TestRules) {
    rules.game_end = Some(vp_policy(EndCheck::RoundEnd, 2));
}

#[test]
fn a_repeating_react_window_is_offered_again_after_each_use() {
    let mut rules = TestRules::new(|s, _| two_boosts(s));
    target_two(&mut rules);
    let mut e = engine(rules, 2);
    use_def(&mut e, P0, 1, 0);
    assert_eq!(
        e.pending().unwrap().options.len(),
        3,
        "decline or use either card"
    );
    e.apply(P0, Action::Decide(1)).unwrap();
    assert_eq!(
        e.pending().unwrap().options.len(),
        2,
        "the used card is turned: decline or the other"
    );
    assert_eq!(vp_of(&e, P0), 1);
    assert_eq!(e.winners(), None);
    e.apply(P0, Action::Decide(1)).unwrap();
    assert_eq!(vp_of(&e, P0), 2);
    assert_eq!(
        e.winners(),
        Some(&[P0][..]),
        "the check ran after the reactions"
    );
}

#[test]
fn declining_the_window_goes_on_to_the_win_check() {
    let mut rules = TestRules::new(|s, _| two_boosts(s));
    target_two(&mut rules);
    let mut e = engine(rules, 2);
    use_def(&mut e, P0, 1, 0);
    e.apply(P0, Action::Decide(1)).unwrap();
    e.apply(P0, Action::Decide(0)).unwrap();
    assert_eq!(vp_of(&e, P0), 1);
    assert_eq!(e.winners(), None);
    assert!(e.pending().is_none());
}

#[test]
fn check_win_without_a_win_condition_does_nothing() {
    let rules = TestRules::new(|s, _| one_card(s, Effect::CheckWin));
    let mut e = engine(rules, 2);
    use_def(&mut e, P0, 1, 0);
    assert_eq!(e.winners(), None);
}

#[test]
fn a_game_can_override_the_winners_hook() {
    #[derive(Clone)]
    struct Fixed(TestRules);
    impl GameRules for Fixed {
        fn setup(&self, players: &[PlayerId], rng: Rng) -> GameState {
            self.0.setup(players, rng)
        }

        fn winners(&self, _state: &GameState) -> Option<Vec<PlayerId>> {
            Some(vec![P1])
        }
    }
    let inner = TestRules::new(|s, _| one_card(s, Effect::CheckWin));
    let mut e = Engine::new(Fixed(inner), 2, 1).unwrap();
    let action =
        e.legal_actions(P0).into_iter().find(|a| matches!(a, Action::Activate { .. })).unwrap();
    e.apply(P0, action).unwrap();
    assert_eq!(e.winners(), Some(&[P1][..]));
}

// ---- extension: hooks, turn order, market, setup order
// ------------------------------------------------------

/// Test rules with the optional hooks filled in.
#[derive(Clone)]
struct Hooked(TestRules);

impl GameRules for Hooked {
    fn setup(&self, players: &[PlayerId], rng: Rng) -> GameState {
        self.0.setup(players, rng)
    }

    fn phase_effects(&self, _state: &GameState, phase: PhaseId) -> Vec<Effect> {
        if phase == PhaseId(1) {
            vec![gain(COIN, 1)]
        } else {
            Vec::new()
        }
    }

    fn pass_effects(&self, state: &GameState, player: PlayerId) -> Vec<Effect> {
        if state.flow.turn_order.passed().first() == Some(&player) {
            vec![vp(1)]
        } else {
            Vec::new()
        }
    }
}

fn hooked(extra: fn(&mut GameState, &[PlayerId])) -> Hooked {
    let mut rules = TestRules::new(extra);
    rules.phases = vec![
        Phase {
            id:          PhaseId(1),
            interactive: false,
        },
        Phase {
            id:          PhaseId(0),
            interactive: true,
        },
    ];
    Hooked(rules)
}

#[test]
fn phase_effects_run_when_a_phase_starts_with_the_first_player_in_control() {
    let e = Engine::new(hooked(nothing), 2, 1).unwrap();
    assert_eq!(coins(&e, P0), Some(1));
    assert_eq!(coins(&e, P1), Some(0));
    assert_eq!(
        e.current_player(),
        Some(P0),
        "the interactive phase follows"
    );
}

#[test]
fn pass_effects_run_for_the_passing_player_before_the_turn_moves_on() {
    let mut e = Engine::new(hooked(nothing), 3, 1).unwrap();
    e.apply(P0, Action::Pass).unwrap();
    assert_eq!(vp_of(&e, P0), 1, "the first to pass gets the bonus");
    assert_eq!(e.current_player(), Some(P1));
    e.apply(P1, Action::Pass).unwrap();
    assert_eq!(vp_of(&e, P1), 0, "later passers do not");
}

#[test]
fn the_first_passer_starts_the_next_round_and_the_others_keep_their_order() {
    let mut rules = TestRules::new(nothing);
    rules.order = TurnOrderPolicy::FirstPasserStarts;
    let mut e = engine(rules, 3);
    end_turn(&mut e);
    e.apply(P1, Action::Pass).unwrap();
    e.apply(P2, Action::Pass).unwrap();
    e.apply(P0, Action::Pass).unwrap();
    assert_eq!(e.state().flow.round, 2);
    assert_eq!(e.state().flow.turn_order.order(), [P1, P2, P0]);
    assert_eq!(e.current_player(), Some(P1));
}

#[test]
fn tasks_pushed_by_setup_run_before_the_first_phase() {
    let rules = TestRules::new(|s, _| {
        s.stack.push(Task::Effect {
            effect: gain(COIN, 2),
            ctx:    Ctx::new(None, P0),
        });
    });
    let e = engine(rules, 2);
    let kinds: Vec<_> = e.events().iter().map(Event::kind).collect();
    assert_eq!(kinds[0], EventKind::ResourceGained);
    assert!(kinds.contains(&EventKind::RoundStarted));
    assert_eq!(coins(&e, P0), Some(2));
}

#[test]
fn a_decision_in_setup_is_answered_before_the_game_starts() {
    let rules = TestRules::new(|s, _| {
        s.stack.push(Task::Effect {
            effect: Effect::Choose(vec![gain(COIN, 1), gain(GEM, 1)]),
            ctx:    Ctx::new(None, P1),
        });
    });
    let mut e = engine(rules, 2);
    assert_eq!(e.pending().unwrap().players, [P1]);
    assert_eq!(e.state().flow.phase, None, "the round has not started");
    assert_eq!(e.current_player(), None);
    e.apply(P1, Action::Decide(1)).unwrap();
    assert_eq!(gems(&e, P1), Some(1));
    assert_eq!(e.current_player(), Some(P0));
}

#[test]
fn the_top_of_the_source_can_be_bought_blind_when_the_policy_allows() {
    let rules = TestRules::new(|s, _| {
        s.markets[0].policy.source_buyable = true;
        s.add_def(card(1, |_| {}));
        put(s, 1, &Owner::Shared, SUPPLY, false);
        put(s, 1, &Owner::Shared, SUPPLY, false);
    });
    let mut e = engine(rules, 2);
    let supply = e.state().zone_id(&Owner::Shared, SUPPLY).unwrap();
    let (below, top) = {
        let cards = e.state().zone(supply).unwrap().entities();
        (cards[0], cards[1])
    };
    let offered = buys(&e, P0);
    assert_eq!(
        offered,
        [Action::Buy {
            entity: top,
            pay:    0,
        }],
        "only the top card"
    );
    e.apply(
        P0,
        Action::Buy {
            entity: top,
            pay:    0,
        },
    )
    .unwrap();
    assert_eq!(zone_of(&e, top), owned(&e, P0, DISCARD));
    // The market refilled from the source, so the next card is now in the row.
    assert_eq!(
        zone_of(&e, below),
        e.state().zone_id(&Owner::Shared, ROW).unwrap()
    );
}

#[test]
fn without_the_policy_the_source_cannot_be_bought_from() {
    let rules = TestRules::new(|s, _| {
        s.add_def(card(1, |_| {}));
        put(s, 1, &Owner::Shared, SUPPLY, false);
    });
    let e = engine(rules, 2);
    assert_eq!(buys(&e, P0).len(), 0);
}

#[test]
fn choose_drops_options_that_cannot_be_paid() {
    let rules = TestRules::new(|s, _| {
        one_card(
            s,
            Effect::Choose(vec![
                Effect::Sequence(vec![Effect::Pay { res: COIN, n: 5 }, gain(GEM, 9)]),
                gain(GEM, 1),
                Effect::Pay { res: COIN, n: 1 },
            ]),
        );
    });
    let mut e = engine(rules, 2);
    use_def(&mut e, P0, 1, 0);
    let pending = e.pending().unwrap();
    assert_eq!(pending.options, [gain(GEM, 1)]);

    let rules =
        TestRules::new(|s, _| one_card(s, Effect::Choose(vec![Effect::Pay { res: COIN, n: 1 }])));
    let mut e = engine(rules, 2);
    use_def(&mut e, P0, 1, 0);
    assert!(e.pending().is_none(), "nothing possible: nothing to decide");
}

#[test]
fn a_view_shows_rivals_components_with_their_resources_and_turned_state() {
    let rules = TestRules::new(|s, _| {
        ra_table(s);
        s.add_def(card(1, |_| {}));
        let c = put(s, 1, &Owner::Player(P1), TABLEAU, true);
        s.gain_entity(c, COIN, 2).unwrap();
        s.entity_mut(c).unwrap().exhausted = true;
        let h = put(s, 1, &Owner::Player(P1), HAND, true);
        s.gain_entity(h, GEM, 1).unwrap();
    });
    let e = engine(rules, 2);
    let view = e.view(P0);
    let tableau =
        view.zones.iter().find(|z| z.kind == TABLEAU && z.owner == Owner::Player(P1)).unwrap();
    assert_eq!(tableau.entities.len(), 1);
    assert_eq!(tableau.entities[0].pool.finite(COIN), Some(2));
    assert!(tableau.entities[0].exhausted);
    let hand = view.zones.iter().find(|z| z.kind == HAND && z.owner == Owner::Player(P1)).unwrap();
    assert_eq!(
        (hand.count, hand.entities.len()),
        (1, 0),
        "a rival's hand stays closed"
    );
}

// ---- extension: gains sized by a paid card, placing from another zone
// ----------------------------------------

fn cost_sized_gain(discarded: bool, bonus: u32) -> Effect {
    Effect::GainChoose {
        to:        Target::Controller,
        n:         Count::CostOf { discarded, bonus },
        exclude:   vec![GEM],
        identical: false,
    }
}

#[test]
fn a_gain_can_be_sized_by_the_placement_cost_of_the_destroyed_card() {
    let rules = TestRules::new(|s, _| {
        ra_table(s);
        s.add_def(card(1, |d| {
            d.abilities = vec![activated(
                Some(Cost::Destroy {
                    which: Which::Other,
                    tag:   "fodder".into(),
                }),
                cost_sized_gain(false, 2),
                Limit::None,
            )];
        }));
        s.add_def(card(2, |d| {
            d.tags = vec!["fodder".into()];
            d.cost = Some(cost_of(2, 1));
        }));
        put(s, 1, &Owner::Player(P0), TABLEAU, true);
        put(s, 2, &Owner::Player(P0), TABLEAU, true);
    });
    let mut e = engine(rules, 2);
    use_def(&mut e, P0, 1, 0);
    // A cost of 3 resources plus a bonus of 2; only coins may be taken, so
    // there is nothing to choose.
    assert!(e.pending().is_none());
    assert_eq!(coins(&e, P0), Some(5));
}

#[test]
fn the_discarded_card_is_bound_separately_from_the_destroyed_one() {
    let rules = TestRules::new(|s, _| {
        ra_table(s);
        s.add_def(card(1, |d| {
            d.tags = vec!["self".into()];
            d.cost = Some(cost_of(4, 0));
            d.abilities = vec![activated(
                Some(Cost::All(vec![
                    Cost::Destroy {
                        which: Which::This,
                        tag:   "self".into(),
                    },
                    Cost::Discard {
                        from: HAND,
                        n:    1,
                    },
                ])),
                cost_sized_gain(true, 0),
                Limit::None,
            )];
        }));
        s.add_def(card(2, |d| d.cost = Some(cost_of(1, 0))));
        put(s, 1, &Owner::Player(P0), TABLEAU, true);
        put(s, 2, &Owner::Player(P0), HAND, true);
    });
    let mut e = engine(rules, 2);
    use_def(&mut e, P0, 1, 0);
    assert_eq!(
        coins(&e, P0),
        Some(1),
        "the discarded card costs 1; the destroyed one (4) is not counted"
    );
}

#[test]
fn place_card_can_take_its_cards_from_another_zone() {
    let rules = TestRules::new(|s, _| {
        ra_table(s);
        s.add_def(card(1, |d| {
            d.abilities = vec![single(Effect::PlaceCard {
                from:     Some(DISCARD),
                tag:      None,
                discount: Some(Discount {
                    tag:     None,
                    n:       1_000,
                    exclude: vec![],
                }),
            })];
        }));
        s.add_def(card(2, |d| d.cost = Some(cost_of(5, 0))));
        put(s, 1, &Owner::Player(P0), TABLEAU, true);
        put(s, 2, &Owner::Player(P0), DISCARD, true);
        put(s, 2, &Owner::Player(P0), HAND, true);
    });
    let mut e = engine(rules, 2);
    use_def(&mut e, P0, 1, 0);
    let pending = e.pending().unwrap();
    assert_eq!(
        pending.options.len(),
        2,
        "the discarded card (free) or nothing; the hand is not offered"
    );
    e.apply(P0, Action::Decide(0)).unwrap();
    let in_play: Vec<_> = e.state().zone(owned(&e, P0, TABLEAU)).unwrap().entities().to_vec();
    assert_eq!(in_play.len(), 2);
    assert_eq!(e.state().zone(owned(&e, P0, DISCARD)).unwrap().len(), 0);
    assert_eq!(e.state().zone(owned(&e, P0, HAND)).unwrap().len(), 1);
}
