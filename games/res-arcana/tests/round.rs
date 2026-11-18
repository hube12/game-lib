//! The Res Arcana rules played through the engine: setup, a scripted round,
//! placing an artifact, and seeded random games that must replay identically.

use gamelib::{
    engine::{Action, Engine},
    id::{PlayerId, ZoneKind},
    zone::Owner,
};
use gamelib_res_arcana::{
    CardKind,
    compile::{ACTIONS, DECK, DISCARD, GOLD, HAND, MONUMENT_DECK, TABLEAU, tag},
    rules::{FIRST_TOKEN, ITEMS, MONUMENT_ROW, PLACES, ResArcanaRules, UNUSED},
};

const P0: PlayerId = PlayerId(0);
const P1: PlayerId = PlayerId(1);

type Game = Engine<ResArcanaRules>;

fn new_game(players: u8, seed: u64) -> Game {
    Engine::new(
        ResArcanaRules::new().with_random_start(false),
        players,
        seed,
    )
    .expect("the game sets up")
}

/// Everybody who may act right now, with what they may do.
fn actors(e: &Game) -> Vec<(PlayerId, Vec<Action>)> {
    e.players()
        .iter()
        .map(|p| (*p, e.legal_actions(*p)))
        .filter(|(_, actions)| !actions.is_empty())
        .collect()
}

/// Answers every pending decision with its first legal option until a player
/// has to take a real action (or the game is over).
fn settle(e: &mut Game) {
    for _ in 0..1_000 {
        if e.winners().is_some() || e.current_player().is_some() && e.pending().is_none() {
            return;
        }
        let Some(pending) = e.pending() else {
            return;
        };
        let who =
            *pending.players.iter().find(|p| pending.waiting_for(**p)).expect("someone waits");
        // The first option that may be taken (a collect step cannot be
        // ended while a collect ability is still to be done).
        let first = e.legal_actions(who).remove(0);
        e.apply(who, first).expect("a legal option");
    }
    panic!("decisions never ended");
}

fn zone_len(e: &Game, owner: &Owner, kind: ZoneKind) -> usize {
    e.state().zone(e.state().zone_id(owner, kind).unwrap()).unwrap().len()
}

fn cards_tagged(e: &Game, owner: &Owner, kind: ZoneKind, wanted: &str) -> usize {
    let zone = e.state().zone(e.state().zone_id(owner, kind).unwrap()).unwrap();
    zone.entities()
        .iter()
        .filter(|id| {
            let def = e.state().entity(**id).unwrap().def;
            e.state().defs[&def].tags.iter().any(|t| t == wanted)
        })
        .count()
}

fn essences(e: &Game, p: PlayerId) -> u32 {
    let pool = e.state().player_pool(p).unwrap();
    gamelib_res_arcana::rules::ESSENCES.iter().map(|r| pool.finite(*r).unwrap_or(0)).sum()
}

fn token(e: &Game, p: PlayerId) -> i64 {
    e.state().counter(FIRST_TOKEN, &Owner::Player(p)).unwrap().value
}

/// A game right after setup, in the first actions phase.
fn started(players: u8, seed: u64) -> Game {
    let mut e = new_game(players, seed);
    settle(&mut e);
    e
}

#[test]
fn the_table_is_set_up_by_the_rulebook() {
    let rules = ResArcanaRules::new().with_random_start(false);
    let expected_mages = rules.defs_of(CardKind::Mage).len();
    assert!(expected_mages > 0);

    let mut e = new_game(2, 7);
    // Both players keep one of their two mages at the same time.
    let pending = e.pending().expect("mages are chosen first");
    assert_eq!(pending.players, [P0, P1]);
    assert_eq!(pending.options.len(), 2);
    assert_eq!(e.current_player(), None, "the game has not started");
    e.apply(P1, Action::Decide(1)).unwrap();
    assert_eq!(
        e.pending().unwrap().players,
        [P0, P1],
        "still waiting for player 0"
    );
    e.apply(P0, Action::Decide(0)).unwrap();

    // Magic items are picked in reverse order: the last player first.
    assert_eq!(e.pending().unwrap().players, [P1]);
    e.apply(P1, Action::Decide(0)).unwrap();
    assert_eq!(e.pending().unwrap().players, [P0]);
    e.apply(P0, Action::Decide(0)).unwrap();
    settle(&mut e);

    assert_eq!(e.current_player(), Some(P0));
    assert_eq!(e.state().flow.phase_id(), Some(ACTIONS));
    for p in [P0, P1] {
        let me = Owner::Player(p);
        assert_eq!(
            cards_tagged(&e, &me, TABLEAU, tag::MAGE),
            1,
            "one mage kept"
        );
        assert_eq!(
            cards_tagged(&e, &me, TABLEAU, tag::ITEM),
            1,
            "one magic item"
        );
        assert_eq!(zone_len(&e, &me, UNUSED), 1, "the other mage is set aside");
        assert_eq!(zone_len(&e, &me, HAND), 3);
        assert_eq!(zone_len(&e, &me, DECK), 5);
        assert!(essences(&e, p) >= 5, "one of each essence to start with");
    }
    assert_eq!(zone_len(&e, &Owner::Shared, MONUMENT_ROW), 2);
    assert_eq!(
        zone_len(&e, &Owner::Shared, MONUMENT_DECK),
        rules.defs_of(CardKind::Monument).len() - 2
    );
    assert_eq!(
        zone_len(&e, &Owner::Shared, PLACES),
        rules.defs_of(CardKind::Place).len()
    );
    assert_eq!(
        zone_len(&e, &Owner::Shared, ITEMS),
        rules.defs_of(CardKind::Item).len() - 2,
        "two items are held"
    );
}

#[test]
fn a_basic_round_discard_pass_pass_victory_check_and_cleanup() {
    let mut e = started(2, 7);
    let (gold_before, hand_before) = (
        e.state().player_pool(P0).unwrap().finite(GOLD).unwrap(),
        zone_len(&e, &Owner::Player(P0), HAND),
    );

    // Player 0's action: discard a card for 1 Gold.
    let card =
        e.state().zone(e.state().zone_id(&Owner::Player(P0), HAND).unwrap()).unwrap().entities()[0];
    assert!(e.legal_actions(P0).contains(&Action::Discard { entity: card }));
    assert!(
        !e.legal_actions(P0).contains(&Action::EndTurn),
        "a turn is exactly one action"
    );
    e.apply(P0, Action::Discard { entity: card }).unwrap();
    assert!(e.pending().is_some(), "1 Gold or any 2 other essences");
    e.apply(P0, Action::Decide(0)).unwrap();
    assert_eq!(
        e.state().player_pool(P0).unwrap().finite(GOLD),
        Some(gold_before + 1)
    );
    assert_eq!(zone_len(&e, &Owner::Player(P0), HAND), hand_before - 1);
    assert_eq!(zone_len(&e, &Owner::Player(P0), DISCARD), 1);
    assert_eq!(e.current_player(), Some(P1), "the action ended the turn");

    // Player 1 passes first: token, a new magic item, one more card.
    let hand_p1 = zone_len(&e, &Owner::Player(P1), HAND);
    e.apply(P1, Action::Pass).unwrap();
    assert!(e.pending().is_some(), "player 1 picks the replacement item");
    settle(&mut e);
    assert_eq!(token(&e, P1), 1);
    assert_eq!(zone_len(&e, &Owner::Player(P1), HAND), hand_p1 + 1);
    assert_eq!(cards_tagged(&e, &Owner::Player(P1), TABLEAU, tag::ITEM), 1);
    assert_eq!(e.current_player(), Some(P0), "a passed player is skipped");

    // Player 0 passes second: no token, but also a card and an item swap.
    e.apply(P0, Action::Pass).unwrap();
    settle(&mut e);
    assert_eq!(token(&e, P0), 0);
    assert_eq!(
        zone_len(&e, &Owner::Player(P0), HAND),
        hand_before,
        "discarded one, drew one"
    );
    assert_eq!(cards_tagged(&e, &Owner::Player(P0), TABLEAU, tag::ITEM), 1);
    assert_eq!(
        zone_len(&e, &Owner::Shared, ITEMS),
        6,
        "swapping keeps the middle the same size"
    );

    // Everybody passed: victory check (nobody has 10), cleanup, round 2.
    assert_eq!(e.winners(), None);
    assert_eq!(e.state().flow.round, 2);
    assert_eq!(
        e.state().flow.turn_order.order(),
        [P1, P0],
        "the first passer starts"
    );
    assert_eq!(e.current_player(), Some(P1));
    assert_eq!(
        token(&e, P1),
        1,
        "the token stays until somebody else passes first"
    );
    assert!(
        e.state().entities().all(|x| !x.exhausted),
        "cleanup straightened everything (and nothing was turned this round)"
    );
}

#[test]
fn an_artifact_can_be_placed_from_hand_paying_its_cost() {
    // Find a deal where player 0 can afford one of the three cards in hand.
    let (mut e, action) = (0..300)
        .find_map(|seed| {
            let e = started(2, seed);
            let place =
                e.legal_actions(P0).into_iter().find(|a| matches!(a, Action::Place { .. }))?;
            Some((e, place))
        })
        .expect("some deal lets the first player place a card");
    let Action::Place { entity, .. } = action else {
        unreachable!()
    };
    let before = essences(&e, P0);
    e.apply(P0, action).unwrap();
    let tableau = e.state().zone_id(&Owner::Player(P0), TABLEAU).unwrap();
    assert!(
        e.state().zone(tableau).unwrap().contains(entity),
        "the card is in play"
    );
    assert!(essences(&e, P0) <= before, "its cost was paid");
    assert_eq!(e.current_player(), Some(P1), "placing is the turn's action");
    // A rival sees which card it is; a card still in hand stays hidden from
    // them.
    let view = e.view(P1);
    let seen = view
        .zones
        .iter()
        .find(|z| z.kind == TABLEAU && z.owner == Owner::Player(P0))
        .and_then(|z| z.entities.iter().find(|c| c.id == entity))
        .expect("the placed card is on the table");
    assert!(seen.def.is_some(), "placed cards are face up to everybody");
    let hand = view.zones.iter().find(|z| z.kind == HAND && z.owner == Owner::Player(P0)).unwrap();
    assert_eq!(hand.entities.len(), 0, "a rival's hand stays closed");
}

#[test]
fn monuments_can_be_claimed_for_four_gold_from_the_row_or_blind_from_the_deck() {
    let mut e = started(2, 3);
    // Not enough gold at the start: nothing to buy yet.
    assert!(!e.legal_actions(P0).iter().any(|a| matches!(a, Action::Buy { .. })));
    // Give player 0 the gold (a test shortcut through the state is not
    // possible; play it instead): discard cards for Gold until 4 are held,
    // passing the turn back and forth.
    let mut guard = 0;
    while e.state().player_pool(P0).unwrap().finite(GOLD).unwrap() < 4 {
        guard += 1;
        assert!(guard < 40, "could not gather the gold");
        let me = e.current_player().unwrap();
        if me == P0 {
            let hand = e
                .state()
                .zone(e.state().zone_id(&Owner::Player(P0), HAND).unwrap())
                .unwrap()
                .entities()
                .to_vec();
            if let Some(card) = hand.first() {
                e.apply(P0, Action::Discard { entity: *card }).unwrap();
                settle(&mut e);
                continue;
            }
        }
        // Somebody has nothing to discard or it is the other player's turn:
        // that player passes the turn by discarding too.
        let hand = e
            .state()
            .zone(e.state().zone_id(&Owner::Player(me), HAND).unwrap())
            .unwrap()
            .entities()
            .to_vec();
        match hand.first() {
            Some(card) => e.apply(me, Action::Discard { entity: *card }).unwrap(),
            None => e.apply(me, Action::Pass).unwrap(),
        };
        settle(&mut e);
    }
    if e.current_player() != Some(P0) {
        // Let the other player act so it is player 0's turn.
        let me = e.current_player().unwrap();
        e.apply(me, Action::Pass).unwrap();
        settle(&mut e);
    }
    let buys: Vec<_> =
        e.legal_actions(P0).into_iter().filter(|a| matches!(a, Action::Buy { .. })).collect();
    assert!(!buys.is_empty(), "4 Gold buys a monument");
    let monuments = e.state().zone_id(&Owner::Shared, MONUMENT_ROW).unwrap();
    let in_row = e.state().zone(monuments).unwrap().entities().to_vec();
    let deck = e.state().zone_id(&Owner::Shared, MONUMENT_DECK).unwrap();
    let blind = e.state().zone(deck).unwrap().top().unwrap();
    assert!(
        buys.iter().any(|a| matches!(a, Action::Buy { entity, .. } if *entity == blind)),
        "the top of the monument deck can be claimed unseen"
    );
    let bought = buys
        .iter()
        .find_map(|a| match a {
            Action::Buy { entity, .. } if in_row.contains(entity) => Some(a.clone()),
            _ => None,
        })
        .expect("a face-up monument is for sale");
    let Action::Buy { entity, .. } = bought else {
        unreachable!()
    };
    e.apply(P0, bought).unwrap();
    let tableau = e.state().zone_id(&Owner::Player(P0), TABLEAU).unwrap();
    assert!(e.state().zone(tableau).unwrap().contains(entity));
    assert_eq!(
        zone_len(&e, &Owner::Shared, MONUMENT_ROW),
        2,
        "the row is refilled"
    );
}

/// A tiny deterministic generator for the bots (the engine's own is inside the
/// state).
struct Lcg(u64);

impl Lcg {
    fn below(&mut self, n: usize) -> usize {
        self.0 =
            self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        usize::try_from(self.0 >> 33).unwrap() % n
    }
}

/// Plays random legal actions (rarely passing) until somebody wins or the
/// budget runs out.
fn play_randomly(e: &mut Game, bot: &mut Lcg, budget: usize) {
    for _ in 0..budget {
        if e.winners().is_some() {
            return;
        }
        let acting = actors(e);
        let Some((player, actions)) = acting.first() else {
            panic!("nobody can act but the game is not over");
        };
        let busy: Vec<_> = actions.iter().filter(|a| **a != Action::Pass).collect();
        let pick = if busy.is_empty() || bot.below(25) == 0 {
            actions[bot.below(actions.len())].clone()
        } else {
            (*busy[bot.below(busy.len())]).clone()
        };
        e.apply(*player, pick).expect("a legal action applies");
    }
}

#[test]
fn random_games_run_without_errors_and_replay_identically() {
    let mut finished = 0;
    for (players, seed) in [(2, 1), (2, 2), (3, 3), (3, 4), (4, 5), (4, 6)] {
        let mut e = new_game(players, seed);
        play_randomly(&mut e, &mut Lcg(seed), 4_000);
        if let Some(winners) = e.winners() {
            finished += 1;
            let rules = ResArcanaRules::new().with_random_start(false);
            println!(
                "{players} players, seed {seed}: round {}, {} actions, winners {winners:?}, \
                 scores {:?}",
                e.state().flow.round,
                e.log().len(),
                gamelib_res_arcana::rules::scoreboard(&rules, e.state())
            );
            assert!(
                winners.iter().all(|w| rules.score(e.state(), *w) >= 10),
                "winners have 10 VP"
            );
        }
        let replayed = Engine::replay(
            ResArcanaRules::new().with_random_start(false),
            players,
            seed,
            e.log(),
        )
        .expect("replays");
        assert_eq!(
            replayed.state(),
            e.state(),
            "seed {seed}: the log reproduces the game"
        );
    }
    println!("{finished}/6 random games reached a winner");
}

#[test]
fn nothing_in_the_log_depends_on_cards_the_compiler_dropped() {
    // The box only contains cards the compiler could express; the rules and the
    // table must agree.
    let rules = ResArcanaRules::new().with_random_start(false);
    let e = started(2, 11);
    for entity in e.state().entities() {
        assert!(
            rules.cards().defs.iter().any(|d| d.id == entity.def),
            "{:?} compiled",
            entity.def
        );
    }
}

#[test]
fn the_first_player_is_rolled_by_dice_and_everybody_can_start() {
    use gamelib::id::PlayerId;
    let mut starters = std::collections::BTreeSet::new();
    for seed in 0..60 {
        let rolls = ResArcanaRules::opening(3, seed);
        let engine = Engine::new(ResArcanaRules::new(), 3, seed).expect("the game sets up");
        let order = engine.state().flow.turn_order.order();
        assert_eq!(order[0], rolls.first, "the highest roll starts");
        // Seats keep their places around the table.
        assert_eq!(order[1].0, (rolls.first.0 + 1) % 3);
        let last = rolls.rounds.last().unwrap();
        let best = last.iter().map(|(_, v)| *v).max().unwrap();
        assert_eq!(
            last.iter().filter(|(_, v)| *v == best).count(),
            1,
            "no tie is left"
        );
        assert!(rolls.rounds.iter().flatten().all(|(_, v)| (1..=6).contains(v)));
        assert_eq!(
            rolls.seat_order(&[PlayerId(0), PlayerId(1), PlayerId(2)])[0],
            rolls.first
        );
        starters.insert(rolls.first.0);
    }
    assert_eq!(starters.len(), 3, "every seat starts sometimes");
}

#[test]
fn magic_items_are_picked_counterclockwise_starting_right_of_the_first_player() {
    for seed in 0..12 {
        let mut e = Engine::new(ResArcanaRules::new(), 4, seed).expect("the game sets up");
        let order = e.state().flow.turn_order.order().to_vec();
        let mut picks = Vec::new();
        while let Some(pending) = e.pending() {
            let who =
                *pending.players.iter().find(|p| pending.waiting_for(**p)).expect("someone waits");
            if pending.players.len() == 1 {
                picks.push(who);
            }
            let first = e.legal_actions(who).remove(0);
            e.apply(who, first).expect("a legal option");
        }
        let mut expected = order.clone();
        expected.reverse();
        picks.truncate(expected.len());
        assert_eq!(picks, expected, "seed {seed}: the last in turn order picks first");
    }
}
