//! Serves the Res Arcana page and its API over HTTP, for developing the web UI
//! in an ordinary browser: `cargo run -p gamelib-res-arcana-gui --features
//! serve --bin res-arcana-dev [port]`, then open `http://127.0.0.1:8787`.
//!
//! Every game is written to its own file (see `gamelib_gui::record`); the
//! directory is printed at start and can be set with `GAMELIB_LOG_DIR`.
//!
//! `res-arcana-dev selfplay [games] [first seed] [players]` plays games
//! between bots instead and logs them the same way.

use gamelib_gui::{NewGame, Request, Response};
use gamelib_res_arcana_gui::{PAGES, backend_with_logs, log_dir};

/// Plays `games` games between bots, each logged to its own file.
fn selfplay(games: u64, first_seed: u64, players: u8) {
    let mut backend = backend_with_logs();
    for seed in first_seed..first_seed + games {
        let answer = backend.handle(Request::NewGame(NewGame {
            players,
            seed: Some(seed),
            humans: Some(Vec::new()),
            options: Vec::new(),
        }));
        match answer {
            Response::Snapshot(snapshot) => {
                let scores: Vec<String> =
                    snapshot.scores.iter().map(|s| format!("P{} {}", s.player, s.value)).collect();
                println!(
                    "seed {seed}: winners {:?}, round {}, {}",
                    snapshot.winners.unwrap_or_default(),
                    snapshot.board.round,
                    scores.join(", ")
                );
            },
            other => println!("seed {seed}: {other:?}"),
        }
    }
}

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    println!("Games are logged to {}", log_dir().display());
    if args.first().map(String::as_str) == Some("selfplay") {
        let number = |i: usize, default: u64| {
            args.get(i).and_then(|a| a.parse::<u64>().ok()).unwrap_or(default)
        };
        selfplay(
            number(1, 1),
            number(2, 1),
            u8::try_from(number(3, 2)).unwrap_or(2),
        );
        return Ok(());
    }
    let port = args.first().cloned().unwrap_or_else(|| "8787".to_owned());
    let addr = format!("127.0.0.1:{port}");
    println!("Res Arcana dev server on http://{addr}");
    // Card art is cut by `scripts/crop_cards.sh` into web/cards (not embedded,
    // not committed).
    let web = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("web");
    gamelib_gui::serve::serve(backend_with_logs(), PAGES, Some(&web), &addr)
}
