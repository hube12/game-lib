//! Planning: a person tries moves out in a what-if copy of the game, with
//! undo and redo, and then plays the list for real.

use gamelib_gui::{Backend, NewGame, Request, Response, Snapshot};
use gamelib_res_arcana_gui::{ResArcanaUi, backend};

fn snapshot(response: Response) -> Snapshot {
    match response {
        Response::Snapshot(s) => *s,
        other => panic!("expected a snapshot, got {other:?}"),
    }
}

fn act(b: &mut Backend<ResArcanaUi>, s: &Snapshot, index: usize) -> Snapshot {
    snapshot(b.handle(Request::Act {
        viewer:   0,
        revision: s.revision,
        action:   index,
    }))
}

/// A game at seat 0's first turn of the actions phase.
fn at_the_first_action(seed: u64) -> (Backend<ResArcanaUi>, Snapshot) {
    let mut b = backend();
    let mut s = snapshot(b.handle(Request::NewGame(NewGame {
        players: 2,
        seed:    Some(seed),
        humans:  Some(vec![0]),
    })));
    while !s.actions.iter().any(|a| a.kind == "pass") {
        let first = s.actions.first().expect("something to decide").index;
        s = act(&mut b, &s, first);
    }
    (b, s)
}

fn pool_total(s: &Snapshot, seat: u8) -> u32 {
    let player = s.board.players.iter().find(|p| p.id == seat).expect("the seat");
    player.pool.iter().map(|a| a.n).sum()
}

fn hand_size(s: &Snapshot) -> usize {
    s.board
        .zones
        .iter()
        .find(|z| z.kind == 1 && z.owner == Some(vec![0]))
        .map_or(0, |z| z.count)
}

/// Discards the first card of the hand and takes whatever is offered first.
fn discard_once(b: &mut Backend<ResArcanaUi>, mut s: Snapshot) -> Snapshot {
    let discard = s.actions.iter().find(|a| a.kind == "discard").expect("a card to discard").index;
    s = act(b, &s, discard);
    while s.actions.iter().any(|a| a.kind == "decide") {
        let first = s.actions[0].index;
        s = act(b, &s, first);
    }
    s
}

#[test]
fn planned_moves_change_only_the_plan_and_can_be_undone_and_redone() {
    let (mut b, real) = at_the_first_action(11);
    assert!(real.plan.is_none());
    let hand = hand_size(&real);
    let essences = pool_total(&real, 0);
    let rival = pool_total(&real, 1);

    let mut s = snapshot(b.handle(Request::PlanStart { viewer: 0 }));
    let plan = s.plan.clone().expect("planning");
    assert!(plan.steps.is_empty() && !plan.can_undo && !plan.can_redo);
    assert_eq!(
        pool_total(&s, 0),
        essences,
        "the plan starts from the real game"
    );

    // Two discards in a row: the rival does not move in between, and the
    // counts show what the planner would have.
    s = discard_once(&mut b, s);
    s = discard_once(&mut b, s);
    assert_eq!(hand_size(&s), hand - 2);
    assert!(pool_total(&s, 0) > essences);
    assert_eq!(pool_total(&s, 1), rival, "rivals do nothing in a plan");
    let plan = s.plan.clone().unwrap();
    assert!(
        plan.steps.len() >= 4,
        "two discards and what they gave: {plan:?}"
    );
    assert!(plan.steps.iter().all(|step| step.done));
    assert!(
        plan.steps[0].label.starts_with("Discard"),
        "{:?}",
        plan.steps[0]
    );
    let planned = plan.steps.len();
    let after_two = pool_total(&s, 0);

    // Undo everything, step by step: back at the start, the list is kept.
    for _ in 0..planned {
        s = snapshot(b.handle(Request::PlanUndo { viewer: 0 }));
    }
    let plan = s.plan.clone().unwrap();
    assert_eq!(plan.steps.len(), planned);
    assert!(plan.steps.iter().all(|step| !step.done) && !plan.can_undo && plan.can_redo);
    assert_eq!((hand_size(&s), pool_total(&s, 0)), (hand, essences));
    assert!(matches!(
        b.handle(Request::PlanUndo { viewer: 0 }),
        Response::Error { .. }
    ));

    // Redo everything.
    for _ in 0..planned {
        s = snapshot(b.handle(Request::PlanRedo { viewer: 0 }));
    }
    assert_eq!((hand_size(&s), pool_total(&s, 0)), (hand - 2, after_two));
    assert!(matches!(
        b.handle(Request::PlanRedo { viewer: 0 }),
        Response::Error { .. }
    ));

    // A new move after an undo replaces what was undone.
    let _ = snapshot(b.handle(Request::PlanUndo { viewer: 0 }));
    s = snapshot(b.handle(Request::PlanUndo { viewer: 0 }));
    let pass = s.actions.iter().find(|a| a.kind == "pass").expect("the second discard is undone");
    s = act(&mut b, &s, pass.index);
    let plan = s.plan.clone().unwrap();
    assert!(!plan.can_redo);
    assert_eq!(plan.steps.last().unwrap().label, "Pass");

    // Dropping the plan: the real game never moved.
    let s = snapshot(b.handle(Request::PlanStop { viewer: 0 }));
    assert!(s.plan.is_none());
    assert_eq!((hand_size(&s), pool_total(&s, 0)), (hand, essences));
    assert_eq!(b.session().unwrap().engine().log().len(), real_log_len(&b));
    assert!(
        s.actions.iter().any(|a| a.kind == "pass"),
        "still the same turn"
    );
}

fn real_log_len(b: &Backend<ResArcanaUi>) -> usize {
    b.session().unwrap().engine().log().len()
}

#[test]
fn a_plan_is_played_for_real_until_a_move_is_no_longer_possible() {
    let (mut b, real) = at_the_first_action(11);
    let before = real_log_len(&b);
    let hand = hand_size(&real);
    let mut s = snapshot(b.handle(Request::PlanStart { viewer: 0 }));
    s = discard_once(&mut b, s);
    s = discard_once(&mut b, s);
    let planned = s.plan.clone().unwrap().steps.len();
    assert_eq!(real_log_len(&b), before, "nothing is played while planning");

    let s = snapshot(b.handle(Request::PlanRun { viewer: 0 }));
    assert!(s.plan.is_none(), "the plan is closed");
    let notice = s.notice.clone().expect("how far it got");
    assert!(
        notice.contains(&format!("all {planned} planned moves")),
        "both discards are still possible after the bot's answers: {notice}"
    );
    assert_eq!(hand_size(&s), hand - 2, "the discards really happened");
    assert!(
        real_log_len(&b) > before + planned - 1,
        "and the bot answered in between"
    );

    // A plan that cannot be played: the moves are simply not made.
    let mut s = snapshot(b.handle(Request::PlanStart { viewer: 0 }));
    if let Some(pass) = s.actions.iter().find(|a| a.kind == "pass") {
        s = act(&mut b, &s, pass.index);
        assert!(s.plan.unwrap().steps.iter().any(|step| step.label == "Pass"));
    }
    let s = snapshot(b.handle(Request::PlanStop { viewer: 0 }));
    assert!(s.notice.is_none());
}

#[test]
fn planning_reveals_no_hidden_card() {
    // Passing draws a card: in the plan it comes from a shuffled copy of the
    // deck, and the real deck is not touched.
    let (mut b, _) = at_the_first_action(11);
    let deck_before: Vec<_> = {
        let state = b.session().unwrap().engine().state();
        let deck = state
            .zone_id(
                &gamelib::zone::Owner::Player(gamelib::id::PlayerId(0)),
                gamelib_res_arcana::compile::DECK,
            )
            .unwrap();
        state.zone(deck).unwrap().entities().to_vec()
    };
    let s = snapshot(b.handle(Request::PlanStart { viewer: 0 }));
    let pass = s.actions.iter().find(|a| a.kind == "pass").unwrap().index;
    let _ = act(&mut b, &s, pass);
    let _ = snapshot(b.handle(Request::PlanStop { viewer: 0 }));
    let state = b.session().unwrap().engine().state();
    let deck = state
        .zone_id(
            &gamelib::zone::Owner::Player(gamelib::id::PlayerId(0)),
            gamelib_res_arcana::compile::DECK,
        )
        .unwrap();
    assert_eq!(state.zone(deck).unwrap().entities(), deck_before);
}

#[test]
fn only_a_person_plans_and_only_one_plan_is_open() {
    let (mut b, _) = at_the_first_action(11);
    assert!(matches!(
        b.handle(Request::PlanStart { viewer: 1 }),
        Response::Error { .. }
    ));
    assert!(matches!(
        b.handle(Request::PlanRun { viewer: 0 }),
        Response::Error { .. }
    ));
    let _ = snapshot(b.handle(Request::PlanStart { viewer: 0 }));
    assert!(matches!(
        b.handle(Request::PlanStart { viewer: 0 }),
        Response::Error { .. }
    ));
}
