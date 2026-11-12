//! Toy deckbuilder used to exercise gamelib.
//!
//! Each player starts with ten Copper and draws five cards per turn. Playing a
//! card (activating its ability) gives coins and moves it to the played pile;
//! coins are spent on cards from a shared row. Estates and Duchies score
//! victory points when bought; the first player to reach [`WIN_VP`] wins on the
//! spot. At the end of a turn the played cards and the hand are discarded and
//! five new cards are drawn. Unspent coins carry over.
#![deny(missing_docs)]
#![warn(clippy::all, clippy::pedantic, clippy::nursery, clippy::cargo)]
#![allow(
    clippy::use_self,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc
)]

use gamelib::{
    card::CardDef,
    cost::Cost,
    counter::Counter,
    effect::{Ability, Condition, Effect, Limit, OwnerRef, Target, Trigger},
    engine::GameRules,
    flow::{EndCheck, Flow, GameEndPolicy, Phase, RoundEndPolicy, TurnOrder, TurnOrderPolicy},
    id::{CounterId, DefId, PhaseId, PlayerId, ResourceId, ZoneKind},
    market::{Market, MarketPolicy, Refill},
    rng::Rng,
    state::GameState,
    zone::{Order, Owner, Visibility},
};

/// Victory points needed to win.
pub const WIN_VP: i64 = 10;
/// Cards in a hand.
pub const HAND_SIZE: u32 = 5;
/// Cards in the market row.
pub const MARKET_SIZE: usize = 5;

/// The only resource.
pub const COIN: ResourceId = ResourceId(0);
/// Victory points.
pub const VP: CounterId = CounterId(0);

/// Player's draw pile.
pub const DECK: ZoneKind = ZoneKind(0);
/// Player's hand.
pub const HAND: ZoneKind = ZoneKind(1);
/// Player's discard pile (recycled into the deck when it runs out).
pub const DISCARD: ZoneKind = ZoneKind(2);
/// Cards played this turn.
pub const PLAYED: ZoneKind = ZoneKind(3);
/// Shared market row.
pub const ROW: ZoneKind = ZoneKind(10);
/// Shared stock the row is refilled from.
pub const SUPPLY: ZoneKind = ZoneKind(11);

/// The single phase of a round.
pub const MAIN: PhaseId = PhaseId(0);

/// Card definition ids.
pub mod cards {
    use gamelib::id::DefId;

    /// Starting card, plays for 1 coin.
    pub const COPPER: DefId = DefId(0);
    /// Costs 3, plays for 2 coins.
    pub const SILVER: DefId = DefId(1);
    /// Costs 6, plays for 3 coins.
    pub const GOLD: DefId = DefId(2);
    /// Costs 2, worth 1 VP when bought.
    pub const ESTATE: DefId = DefId(3);
    /// Costs 5, worth 3 VP when bought.
    pub const DUCHY: DefId = DefId(4);
}

const fn coins(n: u32) -> Effect {
    Effect::Gain {
        to: Target::Controller,
        res: COIN,
        n,
    }
}

const fn vp(delta: i64) -> Effect {
    Effect::ModifyCounter {
        counter: VP,
        owner: OwnerRef::Controller,
        delta,
    }
}

/// Playing a card: gain coins, then move it to the played pile.
fn play(coin_value: u32) -> Ability {
    Ability {
        trigger:   Trigger::Activated,
        condition: Condition::Always,
        cost:      None,
        effect:    Effect::Sequence(vec![
            coins(coin_value),
            Effect::Move {
                entity: Target::Source,
                to:     PLAYED,
                owner:  OwnerRef::Controller,
            },
        ]),
        limit:     Limit::None,
    }
}

fn def(id: DefId, name: &str, price: Option<u32>) -> CardDef {
    CardDef {
        id,
        name: name.into(),
        tags: vec![],
        cost: price.map(|n| Cost::Resources(vec![(COIN, n)])),
        on_gain: None,
        abilities: vec![],
        discounts: vec![],
    }
}

/// All card definitions of the game.
pub fn card_defs() -> Vec<CardDef> {
    let mut copper = def(cards::COPPER, "Copper", None);
    copper.abilities = vec![play(1)];
    let mut silver = def(cards::SILVER, "Silver", Some(3));
    silver.abilities = vec![play(2)];
    let mut gold = def(cards::GOLD, "Gold", Some(6));
    gold.abilities = vec![play(3)];
    let mut estate = def(cards::ESTATE, "Estate", Some(2));
    estate.on_gain = Some(vp(1));
    let mut duchy = def(cards::DUCHY, "Duchy", Some(5));
    duchy.on_gain = Some(vp(3));
    vec![copper, silver, gold, estate, duchy]
}

/// The rules of the toy game.
#[derive(Clone, Copy, Debug, Default)]
pub struct ToyRules;

impl GameRules for ToyRules {
    fn setup(&self, players: &[PlayerId], rng: Rng) -> GameState {
        let mut s = GameState::new(rng);
        s.bank.set_infinite(COIN);
        for def in card_defs() {
            s.add_def(def);
        }

        for &p in players {
            s.add_player(p);
            let owner = Owner::Player(p);
            let deck = s.add_zone(DECK, owner.clone(), Order::Stack, Visibility::Hidden);
            let hand = s.add_zone(HAND, owner.clone(), Order::Unordered, Visibility::Owner);
            s.add_zone(DISCARD, owner.clone(), Order::Stack, Visibility::Public);
            s.add_zone(PLAYED, owner.clone(), Order::Unordered, Visibility::Public);
            s.add_counter(Counter::new(VP, owner, 0, 0, i64::MAX));
            s.zone_mut(deck).expect("zone was just added").recycle_from = Some(DISCARD);
            s.zone_mut(hand).expect("zone was just added").abilities_active = true;
            for _ in 0..10 {
                s.create_entity(cards::COPPER, deck, false).expect("copper is defined");
            }
            s.shuffle(deck).expect("zone was just added");
            s.draw(&Owner::Player(p), DECK, HAND, HAND_SIZE).expect("player zones exist");
        }

        let row = s.add_zone(ROW, Owner::Shared, Order::Row, Visibility::Public);
        let supply = s.add_zone(SUPPLY, Owner::Shared, Order::Stack, Visibility::Hidden);
        for (def, count) in [
            (cards::SILVER, 10),
            (cards::GOLD, 8),
            (cards::ESTATE, 12),
            (cards::DUCHY, 8),
        ] {
            for _ in 0..count {
                s.create_entity(def, supply, false).expect("defs are registered");
            }
        }
        s.shuffle(supply).expect("zone was just added");
        let market = Market {
            zone:   row,
            source: Some(supply),
            size:   MARKET_SIZE,
            dest:   DISCARD,
            policy: MarketPolicy {
                top_only:       false,
                refill:         Refill::Immediate,
                removal:        None,
                source_buyable: false,
            },
        };
        market.refill(&mut s).expect("market zones exist");
        s.markets.push(market);

        s.flow = Flow {
            phases: vec![Phase {
                id:          MAIN,
                interactive: true,
            }],
            turn_order: TurnOrder::new(players.to_vec(), TurnOrderPolicy::Clockwise),
            round_end: RoundEndPolicy::AllPassed,
            game_end: Some(GameEndPolicy {
                counter:  VP,
                target:   WIN_VP,
                when:     EndCheck::AfterEvent,
                tiebreak: vec![],
            }),
            ..Flow::default()
        };
        s
    }

    fn turn_end_effects(&self, _state: &GameState, _player: PlayerId) -> Vec<Effect> {
        let owner = OwnerRef::Controller;
        vec![
            Effect::MoveAll {
                from:  PLAYED,
                to:    DISCARD,
                owner: owner.clone(),
            },
            Effect::MoveAll {
                from:  HAND,
                to:    DISCARD,
                owner: owner.clone(),
            },
            Effect::Draw {
                who:  owner,
                from: DECK,
                to:   HAND,
                n:    HAND_SIZE,
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use gamelib::{
        engine::{Action, Engine},
        event::Event,
    };

    use super::*;

    const P0: PlayerId = PlayerId(0);
    const P1: PlayerId = PlayerId(1);

    fn zone_len(e: &Engine<ToyRules>, owner: &Owner, kind: ZoneKind) -> usize {
        let id = e.state().zone_id(owner, kind).unwrap();
        e.state().zone(id).unwrap().len()
    }

    /// Plays uniformly random legal actions until the game ends.
    fn random_game(seed: u64, players: u8) -> Engine<ToyRules> {
        let mut e = Engine::new(ToyRules, players, seed).unwrap();
        let mut pick = Rng::from_seed(seed ^ 0xA5A5);
        for _ in 0..20_000 {
            if e.winners().is_some() {
                return e;
            }
            let p = e.current_player().unwrap();
            let legal = e.legal_actions(p);
            e.apply(p, legal[pick.below(legal.len())].clone()).unwrap();
        }
        panic!("seed {seed} did not finish");
    }

    #[test]
    fn setup_deals_a_hand_and_fills_the_market() {
        let e = Engine::new(ToyRules, 2, 3).unwrap();
        for p in [P0, P1] {
            let o = Owner::Player(p);
            assert_eq!(zone_len(&e, &o, HAND), 5);
            assert_eq!(zone_len(&e, &o, DECK), 5);
        }
        assert_eq!(zone_len(&e, &Owner::Shared, ROW), MARKET_SIZE);
        assert_eq!(zone_len(&e, &Owner::Shared, SUPPLY), 38 - MARKET_SIZE);
        assert_eq!(e.current_player(), Some(P0));
    }

    #[test]
    fn playing_copper_gives_a_coin_and_moves_the_card() {
        let mut e = Engine::new(ToyRules, 2, 3).unwrap();
        let play = e
            .legal_actions(P0)
            .into_iter()
            .find(|a| matches!(a, Action::Activate { .. }))
            .expect("a hand of copper can be played");
        e.apply(P0, play).unwrap();
        assert_eq!(e.state().player_pool(P0).unwrap().finite(COIN), Some(1));
        assert_eq!(zone_len(&e, &Owner::Player(P0), PLAYED), 1);
        assert_eq!(zone_len(&e, &Owner::Player(P0), HAND), 4);
    }

    #[test]
    fn nothing_is_buyable_without_coins() {
        let e = Engine::new(ToyRules, 2, 3).unwrap();
        assert!(!e.legal_actions(P0).iter().any(|a| matches!(a, Action::Buy { .. })));
    }

    #[test]
    fn ending_the_turn_discards_and_redraws_a_full_hand() {
        let mut e = Engine::new(ToyRules, 2, 3).unwrap();
        while let Some(a) =
            e.legal_actions(P0).into_iter().find(|a| matches!(a, Action::Activate { .. }))
        {
            e.apply(P0, a).unwrap();
        }
        let o = Owner::Player(P0);
        assert_eq!(zone_len(&e, &o, PLAYED), 5);
        e.apply(P0, Action::EndTurn).unwrap();
        assert_eq!(zone_len(&e, &o, HAND), 5);
        assert_eq!(zone_len(&e, &o, PLAYED), 0);
        assert_eq!(zone_len(&e, &o, DISCARD), 5);
        assert_eq!(e.current_player(), Some(P1));
    }

    #[test]
    fn the_deck_is_reshuffled_when_it_runs_out() {
        let mut e = Engine::new(ToyRules, 2, 3).unwrap();
        for _ in 0..4 {
            e.apply(e.current_player().unwrap(), Action::EndTurn).unwrap();
        }
        // Player 0 has now drawn 15 cards from a 10 card deck: it must have
        // recycled the discard pile.
        let o = Owner::Player(P0);
        assert_eq!(
            zone_len(&e, &o, HAND) + zone_len(&e, &o, DECK) + zone_len(&e, &o, DISCARD),
            10
        );
        assert_eq!(zone_len(&e, &o, HAND), 5);
    }

    #[test]
    fn random_games_always_finish_with_a_winner_who_reached_the_target() {
        for seed in 0..25 {
            let e = random_game(seed, 2);
            let winners = e.winners().unwrap();
            assert_eq!(winners.len().min(1), 1, "seed {seed}: no winner");
            for w in winners {
                let score = e.state().counter(VP, &Owner::Player(*w)).unwrap().value;
                assert!(score >= WIN_VP, "seed {seed}: winner has {score} VP");
            }
            assert!(matches!(e.events().last(), Some(Event::GameEnded(_))));
        }
    }

    #[test]
    fn games_with_more_players_finish_too() {
        for players in 3..=4 {
            assert!(random_game(7, players).winners().is_some());
        }
    }

    #[test]
    fn replaying_a_finished_game_gives_the_identical_final_state() {
        for seed in [1, 2, 3] {
            let e = random_game(seed, 2);
            let replay = Engine::replay(ToyRules, 2, seed, e.log()).unwrap();
            assert_eq!(replay.state(), e.state());
            assert_eq!(replay.events(), e.events());
        }
    }

    #[test]
    fn the_same_seed_plays_the_same_game() {
        let (a, b) = (random_game(11, 2), random_game(11, 2));
        assert_eq!(a.log(), b.log());
        assert_eq!(a.winners(), b.winners());
    }

    #[test]
    fn opponents_never_see_a_hand_or_the_deck() {
        let e = Engine::new(ToyRules, 2, 3).unwrap();
        let view = e.view(P1);
        for z in view.zones.iter().filter(|z| z.owner == Owner::Player(P0)) {
            if z.kind == HAND || z.kind == DECK {
                assert!(z.entities.is_empty(), "{:?} leaked", z.kind);
                assert_eq!(z.count, 5);
            }
        }
        let own_hand = e
            .view(P0)
            .zones
            .into_iter()
            .find(|z| z.owner == Owner::Player(P0) && z.kind == HAND)
            .unwrap();
        assert!(own_hand.entities.iter().all(|c| c.def == Some(cards::COPPER)));
    }
}
