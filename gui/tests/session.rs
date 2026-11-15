//! The session and the JSON backend, exercised with the toy game: nothing here
//! knows a real game.

use gamelib::{
    bot::{Bot, RandomBot},
    id::PlayerId,
};
use gamelib_gui::{Backend, GameDescription, GameUi, NewGame, Request, Response, Session};
use gamelib_toy::ToyRules;

#[derive(Clone)]
struct ToyUi;

impl GameUi for ToyUi {
    type Rules = ToyRules;

    fn rules(&self) -> ToyRules {
        ToyRules
    }

    fn description(&self) -> GameDescription {
        GameDescription {
            title:       "Toy".into(),
            min_players: 2,
            max_players: 3,
            resources:   vec![],
            zones:       vec![],
            counters:    vec![],
            phases:      vec![],
            cards:       vec![],
            card_images: String::new(),
        }
    }

    fn bot(&self, _player: PlayerId, seed: u64) -> Box<dyn Bot + Send> {
        Box::new(RandomBot::new(seed))
    }
}

fn session(players: u8, humans: Vec<u8>) -> Session<ToyUi> {
    Session::new(
        ToyUi,
        &NewGame {
            players,
            seed: Some(3),
            humans: Some(humans),
            options: Vec::new(),
        },
        &ToyUi.description(),
    )
    .expect("a game starts")
}

#[test]
fn bots_play_everything_when_nobody_is_human() {
    let s = session(2, vec![]);
    let snapshot = s.snapshot(PlayerId(0));
    assert!(snapshot.winners.is_some(), "the bots finished the game");
    assert!(snapshot.actions.is_empty());
    assert!(snapshot.revision > 0);
    assert!(
        snapshot.log.iter().any(|l| l.text.contains("Game over")),
        "the log tells how it ended"
    );
}

#[test]
fn the_game_waits_for_a_person_and_bots_answer_afterwards() {
    let mut s = session(2, vec![0]);
    let before = s.snapshot(PlayerId(0));
    assert_eq!(before.acting, [0], "only the person has a move");
    assert!(!before.actions.is_empty());
    for (i, a) in before.actions.iter().enumerate() {
        assert_eq!(a.index, i, "indices are positions in the list");
        assert!(!a.label.is_empty());
    }
    let pass =
        before.actions.iter().find(|a| a.kind == "pass").expect("passing is always possible");
    s.act(PlayerId(0), before.revision, pass.index).unwrap();
    let after = s.snapshot(PlayerId(0));
    assert!(after.revision > before.revision);
    // Either it is the person's move again or the game ended; the bot never
    // waits for anybody.
    assert!(after.winners.is_some() || after.acting == [0] || after.acting.is_empty());
}

#[test]
fn stale_wrong_and_foreign_actions_are_refused_without_changing_anything() {
    let mut s = session(2, vec![0]);
    let snap = s.snapshot(PlayerId(0));
    assert!(s.act(PlayerId(0), snap.revision + 5, 0).unwrap_err().contains("moved on"));
    assert!(s.act(PlayerId(0), snap.revision, 9_999).unwrap_err().contains("no such action"));
    assert!(s.act(PlayerId(1), snap.revision, 0).unwrap_err().contains("bot"));
    assert_eq!(s.snapshot(PlayerId(0)), snap);
}

#[test]
fn bad_settings_are_explained() {
    let bad = |players, humans: Vec<u8>| {
        Session::new(
            ToyUi,
            &NewGame {
                players,
                seed: None,
                humans: Some(humans),
                options: Vec::new(),
            },
            &ToyUi.description(),
        )
        .err()
        .expect("refused")
    };
    assert!(bad(5, vec![0]).contains("2 to 3 players"));
    assert!(bad(2, vec![2]).contains("out of range"));
}

#[test]
fn the_json_backend_round_trips_and_reports_errors_as_data() {
    let mut backend = Backend::new(ToyUi);
    let reply = backend.handle_json(r#"{"type":"describe"}"#);
    assert!(reply.contains(r#""type":"description""#));
    assert!(reply.contains(r#""title":"Toy""#));

    assert!(
        backend.handle_json(r#"{"type":"snapshot","viewer":0}"#).contains("no game is running")
    );
    assert!(backend.handle_json("not json").contains("bad request"));

    let started = backend.handle_json(r#"{"type":"new_game","players":2,"seed":7}"#);
    let Response::Snapshot(snapshot) = serde_json::from_str::<Response>(&started).unwrap() else {
        panic!("a snapshot comes back: {started}");
    };
    let act = serde_json::to_string(&Request::Act {
        viewer:   0,
        revision: snapshot.revision,
        action:   0,
    })
    .unwrap();
    assert!(backend.handle_json(&act).contains(r#""type":"snapshot""#));
    assert!(
        backend.handle_json(&act).contains("moved on"),
        "the same revision cannot be played twice"
    );
}

#[test]
fn hidden_cards_never_show_their_identity_to_other_players() {
    let s = session(2, vec![0]);
    let board = s.snapshot(PlayerId(0)).board;
    for zone in &board.zones {
        if zone.visibility == "hidden" {
            assert!(zone.cards.is_empty(), "closed zones list no cards");
        }
        if zone.visibility == "owner" && !zone.owner.as_ref().is_some_and(|o| o.contains(&0)) {
            assert!(
                zone.cards.is_empty(),
                "other players' private zones are closed"
            );
        }
    }
}
