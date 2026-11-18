//! The optional draft variant: everybody is dealt 4 artifacts, keeps 1 and
//! passes the rest to the left until none is left to pass; then 4 more, passed
//! to the right. The 8 kept artifacts become the deck, 3 are drawn, and the
//! mages and magic items are chosen as usual.

use gamelib::{
    engine::{Action, Engine},
    id::{EntityId, PlayerId, ZoneKind},
    zone::Owner,
};
use gamelib_res_arcana::{
    compile::{DECK, HAND},
    rules::{DRAFT, DRAFT_PACK, ResArcanaRules, STOCK},
};

type Game = Engine<ResArcanaRules>;

fn drafted(players: u8, seed: u64) -> Game {
    Engine::new(
        ResArcanaRules::new().with_random_start(false).with_draft(true),
        players,
        seed,
    )
    .expect("the game sets up")
}

fn cards(e: &Game, p: PlayerId, kind: ZoneKind) -> Vec<EntityId> {
    let state = e.state();
    state
        .zone(state.zone_id(&Owner::Player(p), kind).expect("the zone exists"))
        .expect("the zone")
        .entities()
        .to_vec()
}

/// Every player answers the pending decision with `pick(seat, options)`.
fn everybody_picks(e: &mut Game, pick: impl Fn(usize, usize) -> usize) {
    let pending = e.pending().expect("a decision is pending").clone();
    for (seat, p) in pending.players.iter().enumerate() {
        let choice = pick(seat, pending.options.len());
        e.apply(*p, Action::Decide(choice)).expect("a legal option");
    }
}

#[test]
fn the_variant_is_off_by_default() {
    let e = Engine::new(ResArcanaRules::new().with_random_start(false), 2, 1).unwrap();
    assert!(!ResArcanaRules::new().with_random_start(false).draft());
    assert!(e.state().zone_id(&Owner::Player(PlayerId(0)), DRAFT).is_err());
    assert_eq!(
        cards(&e, PlayerId(0), HAND).len(),
        3,
        "the normal game deals the hand at once"
    );
}

#[test]
fn the_draft_starts_with_a_pack_of_four_and_no_hand() {
    let e = drafted(3, 7);
    let pending = e.pending().expect("everybody picks first");
    assert_eq!(pending.players.len(), 3);
    assert_eq!(pending.options.len(), DRAFT_PACK as usize);
    for p in e.players() {
        assert_eq!(cards(&e, *p, DRAFT).len(), 4);
        assert_eq!(cards(&e, *p, STOCK).len(), 4, "the second pack waits");
        assert!(cards(&e, *p, HAND).is_empty());
        assert!(cards(&e, *p, DECK).is_empty());
    }
}

#[test]
fn the_first_half_passes_left_and_the_second_half_passes_right() {
    let mut e = drafted(3, 11);
    let ids = [PlayerId(0), PlayerId(1), PlayerId(2)];

    // First half: everybody keeps their first card; the rest goes to the next
    // seat.
    let packs: Vec<_> = ids.iter().map(|p| cards(&e, *p, DRAFT)).collect();
    everybody_picks(&mut e, |_, _| 0);
    for (i, p) in ids.iter().enumerate() {
        let from = &packs[(i + 2) % 3];
        assert_eq!(
            cards(&e, *p, DRAFT),
            from[1..],
            "seat {i} gets what the seat before it left"
        );
        assert_eq!(
            cards(&e, *p, DECK),
            packs[i][..1],
            "and keeps what it picked"
        );
    }

    // Finish the first half (picks from packs of 3 and 2).
    everybody_picks(&mut e, |_, _| 0);
    everybody_picks(&mut e, |_, _| 0);
    for p in ids {
        assert_eq!(
            cards(&e, p, DECK).len(),
            4,
            "the last card is kept without asking"
        );
        assert!(
            cards(&e, p, DRAFT).len() == 4,
            "and the second pack was dealt"
        );
    }

    // Second half: passed the other way.
    let packs: Vec<_> = ids.iter().map(|p| cards(&e, *p, DRAFT)).collect();
    everybody_picks(&mut e, |_, _| 0);
    for (i, p) in ids.iter().enumerate() {
        let from = &packs[(i + 1) % 3];
        assert_eq!(
            cards(&e, *p, DRAFT),
            from[1..],
            "seat {i} gets what the next seat left"
        );
    }
}

#[test]
fn everybody_ends_with_eight_different_artifacts_and_three_in_hand() {
    for players in 2..=4u8 {
        let mut e = drafted(players, 3);
        let mut rounds = 0;
        // Pick a different card each time; stop when the packs are used up (the
        // mages are next).
        while !cards(&e, PlayerId(0), DRAFT).is_empty() {
            everybody_picks(&mut e, |seat, options| (seat + options - 1) % options);
            rounds += 1;
            assert!(rounds <= 6, "the draft is six picks long");
        }
        assert_eq!(
            rounds, 6,
            "3 picks per half, the 4th card is kept without asking"
        );
        let mut all = Vec::new();
        for p in e.players() {
            assert_eq!(cards(&e, *p, HAND).len(), 3);
            assert_eq!(cards(&e, *p, DECK).len(), 5);
            assert!(cards(&e, *p, DRAFT).is_empty());
            assert!(cards(&e, *p, STOCK).is_empty());
            all.extend(cards(&e, *p, HAND));
            all.extend(cards(&e, *p, DECK));
        }
        let total = all.len();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), total, "no artifact is dealt twice");
        assert_eq!(total, 8 * usize::from(players));
        let pending = e.pending().expect("the mages are next");
        assert_eq!(
            pending.options.len(),
            2,
            "then everybody keeps one of two mages"
        );
    }
}

#[test]
fn a_drafted_game_replays_identically() {
    let rules = ResArcanaRules::new().with_random_start(false).with_draft(true);
    let mut e = Engine::new(rules.clone(), 2, 5).unwrap();
    for _ in 0..40 {
        let Some(pending) = e.pending() else { break };
        let who = *pending.players.iter().find(|p| pending.waiting_for(**p)).unwrap();
        let first = e.legal_actions(who).remove(0);
        e.apply(who, first).unwrap();
    }
    let replay = Engine::replay(rules, 2, 5, e.log()).expect("the log replays");
    assert_eq!(replay.state(), e.state());
}
