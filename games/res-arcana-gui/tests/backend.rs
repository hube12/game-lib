//! Res Arcana through the GUI backend: a person plays by action index, the
//! bots answer, and nothing hidden leaks into what the person sees.

use gamelib::{effect::Effect, engine::Engine, id::PlayerId};
use gamelib_gui::{
    GameUi,
    Names,
    Request,
    Response,
    Snapshot,
    describe::{Describer, TERM_END, TERM_SEP, TERM_START, plain},
};
use gamelib_res_arcana::{
    CardKind,
    compile::HAND,
    rules::{DRAFT, ResArcanaRules},
};
use gamelib_res_arcana_gui::{ResArcanaUi, backend};

fn snapshot(response: Response) -> Snapshot {
    match response {
        Response::Snapshot(s) => *s,
        other => panic!("expected a snapshot, got {other:?}"),
    }
}

fn start(players: u8, seed: u64) -> (gamelib_gui::Backend<ResArcanaUi>, Snapshot) {
    let mut b = backend();
    let s = snapshot(b.handle(Request::NewGame(gamelib_gui::NewGame {
        players,
        seed: Some(seed),
        humans: Some(vec![0]),
        options: Vec::new(),
    })));
    (b, s)
}

/// What a simple person does: claim and place when possible, discard a big
/// hand, otherwise pass.
fn pick(s: &Snapshot) -> usize {
    let hand: usize = s
        .board
        .zones
        .iter()
        .filter(|z| z.kind == HAND.0 && z.owner == Some(vec![0]))
        .map(|z| z.count)
        .sum();
    let first = |kind: &str| s.actions.iter().find(|a| a.kind == kind).map(|a| a.index);
    first("decide")
        .or_else(|| first("buy"))
        .or_else(|| first("place"))
        .or_else(|| if hand > 4 { first("discard") } else { None })
        .or_else(|| first("pass"))
        .expect("a person always has something to do")
}

#[test]
fn a_person_plays_through_the_backend_while_bots_answer() {
    for players in [2, 3, 4] {
        let (mut b, mut s) = start(players, u64::from(players));
        assert_eq!(s.board.players.len(), usize::from(players));
        assert!(s.board.players[0].human && !s.board.players[1].human);
        let mut last = s.revision;
        for _ in 0..400 {
            if s.winners.is_some() {
                break;
            }
            assert_eq!(s.acting, [0], "bots never leave the game waiting for them");
            let index = pick(&s);
            s = snapshot(b.handle(Request::Act {
                viewer:   0,
                revision: s.revision,
                action:   index,
            }));
            assert!(s.revision > last, "every action moves the revision on");
            last = s.revision;
        }
        assert!(
            s.winners.is_some() || last > 50,
            "the game progressed ({players} players)"
        );
        assert!(!s.log.is_empty());
    }
}

#[test]
fn the_first_decisions_name_the_mages_and_the_card_actions_name_their_cards() {
    let (mut b, s) = start(2, 1);
    assert_eq!(s.actions.len(), 2, "two mages to keep");
    assert!(s.actions.iter().all(|a| a.kind == "decide" && a.label.contains("move ")));
    assert!(
        s.actions.iter().all(|a| !a.label.contains("card 1") && !a.label.contains("card 2")),
        "the mages are named: {:?}",
        s.actions
    );
    // Each option shows the mage it keeps as a real card, readable to its
    // owner.
    let mut shown = Vec::new();
    for a in &s.actions {
        assert_eq!(a.cards.len(), 1, "one mage per option: {:?}", a.cards);
        assert!(a.cards[0].def.is_some(), "my own dealt mage is readable");
        shown.push(a.cards[0].id);
    }
    shown.dedup();
    assert_eq!(shown.len(), 2, "the two options show different mages");
    let s = snapshot(b.handle(Request::Act {
        viewer:   0,
        revision: s.revision,
        action:   0,
    }));
    // Then the magic item pick: one option per item, each named.
    assert!(s.actions.iter().all(|a| a.kind == "decide"));
    assert!(s.actions.len() >= 6);
    assert!(
        s.actions.iter().all(|a| !a.cards.is_empty() && a.cards.iter().all(|c| c.def.is_some())),
        "every magic item on offer is shown as a card"
    );
    let s = snapshot(b.handle(Request::Act {
        viewer:   0,
        revision: s.revision,
        action:   0,
    }));
    let kinds: Vec<_> = s.actions.iter().map(|a| a.kind.as_str()).collect();
    assert!(
        kinds.contains(&"pass") && kinds.contains(&"discard"),
        "{kinds:?}"
    );
    assert!(!kinds.contains(&"end_turn"));
    for a in s.actions.iter().filter(|a| a.kind == "discard") {
        assert!(a.label.starts_with("Discard "));
    }
}

#[test]
fn rivals_hands_and_decks_stay_closed_and_counts_stay_visible() {
    let (_, s) = start(2, 5);
    for zone in &s.board.zones {
        if zone.owner == Some(vec![1])
            && (zone.kind == 0 || zone.kind == 1 || zone.kind == 14 || zone.kind == 15)
        {
            assert!(
                zone.cards.is_empty(),
                "zone {} of the rival is closed",
                zone.kind
            );
        }
    }
    let rival_hand =
        s.board.zones.iter().find(|z| z.kind == HAND.0 && z.owner == Some(vec![1])).unwrap();
    assert_eq!(rival_hand.count, 3, "the number of cards in hand is public");
    let my_hand =
        s.board.zones.iter().find(|z| z.kind == HAND.0 && z.owner == Some(vec![0])).unwrap();
    assert!(
        my_hand.cards.iter().all(|c| c.def.is_some()),
        "my own hand is readable"
    );
}

#[test]
fn the_describer_hides_the_names_of_cards_the_viewer_cannot_see() {
    let ui = ResArcanaUi::new();
    let names = Names::new(&ui.description());
    let engine = Engine::new(ResArcanaRules::new(), 2, 9).unwrap();
    let state = engine.state();
    let rules = ResArcanaRules::new();
    let artifacts = rules.defs_of(CardKind::Artifact);
    assert!(!artifacts.is_empty());

    let zone_cards = |owner: u8, kind| {
        let z = state.zone_id(&gamelib::zone::Owner::Player(PlayerId(owner)), kind).unwrap();
        state.zone(z).unwrap().entities().to_vec()
    };
    let mine = zone_cards(0, HAND)[0];
    let theirs = zone_cards(1, HAND)[0];
    let deck_top = zone_cards(1, gamelib_res_arcana::compile::DECK)[0];
    let monument_deck = state
        .zone_id(
            &gamelib::zone::Owner::Shared,
            gamelib_res_arcana::compile::MONUMENT_DECK,
        )
        .unwrap();
    let blind = state.zone(monument_deck).unwrap().top().unwrap();

    let d = Describer::for_viewer(state, &names, PlayerId(0));
    assert_ne!(
        d.entity(mine),
        "a hidden card",
        "my own hand is mine to read"
    );
    assert_eq!(d.entity(theirs), "a hidden card");
    assert_eq!(d.entity(deck_top), "a hidden card");
    assert_eq!(
        d.entity(blind),
        "a hidden card",
        "claiming blind from the monument deck reveals nothing"
    );
    // Options about a card show it as a card only when the viewer may know
    // which card it is.
    let about = |card| Effect::WithCard {
        card,
        effect: Box::new(Effect::Ignore),
    };
    assert!(d.cards_in(&about(mine))[0].def.is_some());
    assert_eq!(
        d.cards_in(&about(theirs))[0].def,
        None,
        "a rival's hand card stays a card back"
    );
    assert_eq!(d.cards_in(&about(blind))[0].def, None);
    assert_eq!(
        d.cards_in(&Effect::Sequence(vec![
            about(mine),
            about(mine),
            Effect::Ignore
        ]))
        .len(),
        1,
        "no repeats"
    );
    // A tool that is not a viewer (tests, logs for the host) sees everything.
    assert_ne!(
        Describer::new(state, &names).entity(theirs),
        "a hidden card"
    );
}

#[test]
fn the_backend_can_live_behind_a_mutex_shared_between_threads() {
    // The desktop shell keeps it in managed state, which needs `Send`.
    fn assert_send<T: Send>() {}
    assert_send::<gamelib_gui::Backend<ResArcanaUi>>();
    let shared = std::sync::Mutex::new(backend());
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let reply = shared.lock().unwrap().handle_json(r#"{"type":"describe"}"#);
            assert!(reply.contains("Res Arcana"));
        });
    });
}

#[test]
fn labels_mark_their_terms_so_the_page_can_quote_underline_and_explain_them() {
    let (_, s) = start(2, 1);
    for a in &s.actions {
        // The plain label is the marked-up text without its markers, and has
        // none itself.
        assert_eq!(a.label, plain(&a.markup));
        assert!(!a.label.contains([TERM_START, TERM_SEP, TERM_END]));
        // "move <mage> from <zone> to <zone>": one card term and two zone
        // terms.
        let terms = |kind: &str| a.markup.matches(&format!("{TERM_START}{kind}:")).count();
        assert_eq!(terms("card"), 1, "{:?}", a.markup);
        assert_eq!(terms("zone"), 4, "{:?}", a.markup);
    }
    for line in &s.log {
        assert_eq!(line.text, plain(&line.markup));
    }
}

#[test]
fn every_term_has_a_help_text_for_its_popup() {
    let d = ResArcanaUi::new().description();
    for r in &d.resources {
        assert!(!r.help.is_empty(), "resource {}", r.name);
    }
    for z in d.zones.iter().chain(&d.counters).chain(&d.phases) {
        assert!(!z.help.is_empty(), "{}", z.name);
    }
}

#[test]
fn payments_are_described_with_marked_resources_too() {
    // Find a game where the first player can place a card, then look at how its
    // payment reads.
    let (_, s) = (0..300u64)
        .find_map(|seed| {
            let (mut b, mut s) = start(2, seed);
            for _ in 0..2 {
                s = snapshot(b.handle(Request::Act {
                    viewer:   0,
                    revision: s.revision,
                    action:   0,
                }));
            }
            s.actions.iter().any(|a| a.kind == "place" && a.detail != "free").then_some((b, s))
        })
        .expect("some deal lets the first player place a card");
    for a in s.actions.iter().filter(|a| a.kind == "place") {
        assert_eq!(a.detail, plain(&a.detail_markup));
        if a.detail != "free" {
            assert!(
                a.detail_markup.contains(&format!("{TERM_START}res:")),
                "{:?}",
                a.detail_markup
            );
        }
        assert!(!a.detail.contains([TERM_START, TERM_SEP, TERM_END]));
    }
}

#[test]
fn a_choice_of_resources_is_described_as_a_mix_for_the_picker() {
    // Discard a card, choose "any 2 other essences": the options are all the
    // mixes of 2.
    let (mut b, mut s) = start(2, 1);
    for _ in 0..2 {
        s = snapshot(b.handle(Request::Act {
            viewer:   0,
            revision: s.revision,
            action:   0,
        }));
    }
    let discard = s.actions.iter().find(|a| a.kind == "discard").expect("a card to discard").index;
    let s = snapshot(b.handle(Request::Act {
        viewer:   0,
        revision: s.revision,
        action:   discard,
    }));
    assert_eq!(s.actions.len(), 2, "1 Gold or any 2 other essences");
    assert_eq!(
        s.actions[0].mix.as_ref().map(|m| m.direction.as_str()),
        Some("gain")
    );
    assert!(
        s.actions[1].mix.is_none(),
        "the second option opens another choice"
    );
    let s = snapshot(b.handle(Request::Act {
        viewer:   0,
        revision: s.revision,
        action:   1,
    }));
    // Mixes of 2 over the four kinds that are not Gold.
    assert_eq!(s.actions.len(), 10);
    for a in &s.actions {
        let mix = a.mix.as_ref().expect("every option is a mix");
        assert_eq!(mix.direction, "gain");
        assert_eq!(mix.amounts.iter().map(|m| m.n).sum::<u32>(), 2);
    }
}

// ---------------------------------------------------------------------------------------------------
// The collect step on the cards, and looking at the top of a deck
// ---------------------------------------------------------------------------------------------------

fn act(b: &mut gamelib_gui::Backend<ResArcanaUi>, s: &Snapshot, index: usize) -> Snapshot {
    snapshot(b.handle(Request::Act {
        viewer:   0,
        revision: s.revision,
        action:   index,
    }))
}

fn card_name(b: &gamelib_gui::Backend<ResArcanaUi>, def: u32) -> String {
    b.description().cards.iter().find(|c| c.id == def).expect("a described card").name.clone()
}

/// A game in which seat 0 keeps the Seer, right after the setup choices.
fn game_with_the_seer() -> (gamelib_gui::Backend<ResArcanaUi>, Snapshot) {
    for seed in 1..400 {
        let (mut b, s) = start(2, seed);
        let seer = s.actions.iter().find(|a| {
            a.kind == "decide"
                && a.cards.iter().any(|c| c.def.is_some_and(|d| card_name(&b, d) == "Seer"))
        });
        let Some(seer) = seer else {
            continue;
        };
        let mut s = act(&mut b, &s, seer.index);
        // The magic item: any.
        while s.actions.iter().any(|a| a.kind == "decide" && !a.skip && a.entity.is_none()) {
            s = act(&mut b, &s, s.actions[0].index);
        }
        return (b, s);
    }
    panic!("no seed deals the Seer to seat 0");
}

#[test]
fn the_collect_step_is_offered_on_the_cards_and_ends_with_done() {
    let (mut b, mut s) = game_with_the_seer();
    // The Seer collects 1 Calm: the option belongs to her card, says what it
    // does, and the step cannot be ended before it is done.
    let collect: Vec<_> = s.actions.iter().filter(|a| a.kind == "decide").collect();
    assert!(
        !collect.is_empty(),
        "the collect step is pending: {:?}",
        s.actions
    );
    assert!(
        collect.iter().all(|a| !a.skip),
        "nothing to skip while a collect is owed"
    );
    let seer = collect
        .iter()
        .find(|a| a.short.contains("gain 1") && a.short.contains("Calm"))
        .unwrap_or_else(|| panic!("the Seer's collect: {collect:?}"));
    let on_board = s.board.zones.iter().flat_map(|z| &z.cards).any(|c| Some(c.id) == seer.entity);
    assert!(on_board, "the option names a card the page shows");
    let calm_before = pool_of(&s, 2);
    s = act(&mut b, &s, seer.index);
    assert_eq!(pool_of(&s, 2), calm_before + 1);
    // Whatever else is owed, then the actions begin without a "Done" when
    // nothing lies on the components.
    while let Some(next) = s.actions.iter().find(|a| a.kind == "decide") {
        s = act(&mut b, &s, next.index);
    }
    assert!(
        s.actions.iter().any(|a| a.kind == "pass"),
        "the actions phase: {:?}",
        s.actions
    );
}

/// How many of a resource seat 0 holds.
fn pool_of(s: &Snapshot, res: u16) -> u32 {
    let me = s.board.players.iter().find(|p| p.id == 0).expect("seat 0");
    me.pool.iter().find(|a| a.res == res).map_or(0, |a| a.n)
}

#[test]
fn looking_at_the_top_of_a_deck_shows_the_cards_to_the_player_who_looks() {
    let (mut b, mut s) = game_with_the_seer();
    while let Some(next) = s.actions.iter().find(|a| a.kind == "decide") {
        s = act(&mut b, &s, next.index);
    }
    let look = s
        .actions
        .iter()
        .find(|a| a.kind == "activate" && a.label.contains("look at the top 3"))
        .unwrap_or_else(|| panic!("the Seer's power: {:?}", s.actions));
    s = act(&mut b, &s, look.index);
    // Her own deck or the monument deck.
    assert_eq!(s.actions.len(), 2, "{:?}", s.actions);
    let monuments = s
        .actions
        .iter()
        .find(|a| a.label.contains("Monument deck"))
        .unwrap_or_else(|| panic!("the monument deck: {:?}", s.actions));
    s = act(&mut b, &s, monuments.index);
    // Six orders; each shows the same three monuments, face up, top first.
    assert_eq!(s.actions.len(), 6);
    let mut seen: Vec<Vec<String>> = Vec::new();
    for option in &s.actions {
        assert!(
            option.label.starts_with("put back, top first: "),
            "{}",
            option.label
        );
        let names: Vec<String> = option
            .cards
            .iter()
            .map(|c| card_name(&b, c.def.expect("the looked-at card is shown")))
            .collect();
        assert_eq!(names.len(), 3);
        assert_eq!(
            option.label,
            format!("put back, top first: {}", names.join(", "))
        );
        seen.push(names);
    }
    let mut first = seen[0].clone();
    first.sort();
    for names in &seen {
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(sorted, first, "always the same three cards");
    }
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), 6, "six different orders");
    // The pile itself stays hidden on the board.
    let deck = s.board.zones.iter().find(|z| z.kind == 10).expect("the monument deck");
    assert!(deck.cards.is_empty());
}

#[test]
fn without_auto_next_the_bots_wait_for_next_and_a_move_can_be_undone() {
    let (mut b, s) = start(2, 1);
    assert!(s.auto && !s.awaiting_next && !s.can_undo);
    let s = snapshot(b.handle(Request::SetAuto { auto: false, viewer: 0 }));
    assert!(!s.auto);
    // The mage draft is the first thing to do; play until the first move
    // after which a bot is due to move.
    let mut s = s;
    let mut guard = 0;
    while !s.awaiting_next {
        guard += 1;
        assert!(guard < 20, "a bot becomes due after a person's move");
        let index = pick(&s);
        s = snapshot(b.handle(Request::Act { viewer: 0, revision: s.revision, action: index }));
    }
    assert!(s.actions.is_empty(), "nothing to do while the next player waits");
    assert!(s.can_undo);
    let waiting = s.revision;
    // Undo takes the person's turn back; they can act again.
    let undone = snapshot(b.handle(Request::Undo { viewer: 0 }));
    assert!(undone.revision > waiting && !undone.awaiting_next && !undone.actions.is_empty());
    assert!(!undone.can_undo);
    // Play again, then press Next: the bot moves and it is the person's turn.
    let index = pick(&undone);
    let mut s = snapshot(b.handle(Request::Act { viewer: 0, revision: undone.revision, action: index }));
    while !s.awaiting_next {
        let index = pick(&s);
        s = snapshot(b.handle(Request::Act { viewer: 0, revision: s.revision, action: index }));
    }
    let after = snapshot(b.handle(Request::Next { viewer: 0 }));
    assert!(!after.awaiting_next && !after.can_undo);
    assert_eq!(after.acting, [0]);
    // Turning auto back on lets a waiting bot move at once.
    let mut s = after;
    while !s.awaiting_next {
        let index = pick(&s);
        s = snapshot(b.handle(Request::Act { viewer: 0, revision: s.revision, action: index }));
    }
    let s = snapshot(b.handle(Request::SetAuto { auto: true, viewer: 0 }));
    assert!(s.auto && !s.awaiting_next);
    assert_eq!(s.acting, [0]);
}

#[test]
fn planning_never_moves_a_bot_and_waiting_bots_hold_until_it_is_left() {
    let (mut b, _) = start(2, 1);
    let mut s = snapshot(b.handle(Request::SetAuto { auto: false, viewer: 0 }));
    while !s.awaiting_next {
        let index = pick(&s);
        s = snapshot(b.handle(Request::Act { viewer: 0, revision: s.revision, action: index }));
    }
    let _ = s;
    // A bot is due: planning is refused until Next, and Next/Undo are
    // refused while a plan is open.
    assert!(matches!(b.handle(Request::PlanStart { viewer: 0 }), Response::Error { .. }));
    let s = snapshot(b.handle(Request::Next { viewer: 0 }));
    let log_before = s.log.clone();
    let planning = snapshot(b.handle(Request::PlanStart { viewer: 0 }));
    assert!(planning.plan.is_some() && !planning.awaiting_next && !planning.can_undo);
    let index = pick(&planning);
    let planned = snapshot(b.handle(Request::Act {
        viewer: 0,
        revision: planning.revision,
        action: index,
    }));
    assert!(planned.plan.is_some());
    assert!(matches!(b.handle(Request::Next { viewer: 0 }), Response::Error { .. }));
    assert!(matches!(b.handle(Request::Undo { viewer: 0 }), Response::Error { .. }));
    // Leaving the plan shows the real game, where nothing moved.
    let left = snapshot(b.handle(Request::PlanStop { viewer: 0 }));
    assert_eq!(left.log, log_before, "the bot did not move while planning");
    assert_eq!(left.acting, [0]);
}

fn pack(s: &Snapshot, seat: u8) -> &gamelib_gui::board::ZoneDto {
    s.board.zones.iter().find(|z| z.kind == DRAFT.0 && z.owner == Some(vec![seat])).unwrap()
}

#[test]
fn the_draft_option_deals_a_pack_to_pick_from_and_hides_the_rivals_packs() {
    let mut b = backend();
    let s = snapshot(b.handle(Request::NewGame(gamelib_gui::NewGame {
        players: 3,
        seed:    Some(2),
        humans:  Some(vec![0]),
        options: vec![gamelib_res_arcana_gui::DRAFT_OPTION.to_owned()],
    })));
    assert_eq!(pack(&s, 0).cards.len(), 4, "the viewer sees their own pack");
    assert!(
        pack(&s, 1).cards.iter().all(|c| c.def.is_none()),
        "and nobody else's"
    );
    let picks: Vec<_> = s.actions.iter().filter(|a| a.kind == "decide").collect();
    assert_eq!(picks.len(), 4, "one pick per card of the pack");

    // Picking passes the rest on: the viewer's pack is now what a rival left.
    let s = snapshot(b.handle(Request::Act {
        viewer:   0,
        revision: s.revision,
        action:   picks[0].index,
    }));
    assert_eq!(pack(&s, 0).cards.len(), 3);

    // Without the option nothing changes.
    let (_, plain) = start(3, 2);
    assert!(plain.board.zones.iter().all(|z| z.kind != DRAFT.0));
}
