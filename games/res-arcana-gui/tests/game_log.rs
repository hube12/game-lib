//! Every game played through the backend is written to its own file.

use std::{fs, path::PathBuf};

use gamelib_gui::{NewGame, Request, Response};
use gamelib_res_arcana_gui::backend;
use serde_json::Value;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("res-arcana-log-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    dir
}

fn lines(dir: &PathBuf) -> Vec<Value> {
    let mut files: Vec<_> = fs::read_dir(dir).unwrap().map(|f| f.unwrap().path()).collect();
    assert_eq!(files.len(), 1, "one file per game");
    let text = fs::read_to_string(files.remove(0)).unwrap();
    text.lines().map(|l| serde_json::from_str(l).expect("every line is JSON")).collect()
}

fn kind(line: &Value) -> &str {
    line["type"].as_str().unwrap_or_default()
}

#[test]
fn a_game_between_bots_is_logged_from_start_to_end() {
    let dir = scratch("bots");
    let mut backend = backend().with_log_dir(&dir);
    let answer = backend.handle(Request::NewGame(NewGame {
        players: 2,
        seed:    Some(3),
        humans:  Some(Vec::new()),
    }));
    let Response::Snapshot(snapshot) = answer else {
        panic!("the game starts: {answer:?}");
    };
    assert!(snapshot.winners.is_some(), "bots play to the end");

    let log = lines(&dir);
    assert_eq!(kind(&log[0]), "start");
    assert_eq!(log[0]["seed"], 3);
    assert_eq!(kind(log.last().unwrap()), "end");
    assert_eq!(kind(&log[1]), "round", "the table at the start");
    let actions: Vec<&Value> = log.iter().filter(|l| kind(l) == "action").collect();
    assert_eq!(
        actions.len() as u64,
        log.last().unwrap()["actions"].as_u64().unwrap()
    );
    // Every move says who did what in words, carries the raw action for a
    // replay, and a bot explains itself.
    for (i, action) in actions.iter().enumerate() {
        assert_eq!(action["n"].as_u64().unwrap() as usize, i + 1);
        assert!(
            action["label"].as_str().is_some_and(|l| !l.is_empty()),
            "{action}"
        );
        assert!(!action["action"].is_null());
        assert_eq!(action["human"], false);
        assert!(action["bot"].as_str().is_some(), "{action}");
        assert!(action["scores"]["0"].is_i64() && action["pools"]["1"].is_object());
    }
    let rounds = log.iter().filter(|l| kind(l) == "round").count() as u64;
    assert_eq!(
        rounds,
        log.last().unwrap()["round"].as_u64().unwrap() + 1,
        "the setup (round 0) and one line per round"
    );
    let winner = log.last().unwrap()["winners"][0].as_u64().unwrap().to_string();
    assert!(log.last().unwrap()["scores"][&winner].as_i64().unwrap() >= 10);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_persons_moves_are_logged_too_and_no_log_dir_means_no_files() {
    let dir = scratch("person");
    let mut backend = backend().with_log_dir(&dir);
    let new_game = || {
        Request::NewGame(NewGame {
            players: 2,
            seed:    Some(5),
            humans:  Some(vec![0]),
        })
    };
    let Response::Snapshot(mut snapshot) = backend.handle(new_game()) else {
        panic!("the game starts");
    };
    for _ in 0..12 {
        let Some(action) = snapshot.actions.first() else {
            break;
        };
        let answer = backend.handle(Request::Act {
            viewer:   0,
            revision: snapshot.revision,
            action:   action.index,
        });
        let Response::Snapshot(next) = answer else {
            panic!("a legal move is accepted: {answer:?}");
        };
        snapshot = next;
    }
    let log = lines(&dir);
    let mine = log.iter().filter(|l| kind(l) == "action" && l["human"] == true).count();
    assert_eq!(mine, 12);
    assert!(log.iter().any(|l| kind(l) == "action" && l["human"] == false));
    fs::remove_dir_all(dir).unwrap();

    // The plain backend (tests, tools) writes nothing.
    assert!(gamelib_res_arcana_gui::backend().log_dir().is_none());
}
