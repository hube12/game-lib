//! Headless runner: plays games between bots and checks that every game replays
//! identically.

use std::process::ExitCode;

use gamelib::engine::{Engine, GameRules};
use gamelib_cli::{
    bot::{Bot, RandomBot},
    runner::{Outcome, RunError, run},
};
use gamelib_res_arcana::{
    bot::HeuristicBot,
    planner::{Difficulty, Params, PlannerBot},
    rules::ResArcanaRules,
};
use gamelib_toy::ToyRules;

/// Which game to play.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Game {
    #[default]
    Toy,
    ResArcana,
}

/// Which bot sits at every seat.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum BotKind {
    #[default]
    Random,
    Heuristic,
    /// The planner with its numbers; `tuned` when they are not the defaults.
    Planner {
        params: Params,
        tuned:  bool,
        level:  Difficulty,
    },
}

impl BotKind {
    fn parse(name: &str) -> Result<Self, String> {
        match name {
            "random" => Ok(Self::Random),
            "heuristic" => Ok(Self::Heuristic),
            other => Self::planner(other).ok_or_else(|| format!("unknown bot {other}\n{USAGE}")),
        }
    }

    /// `planner`, or `planner:name=value:name=value` to change its numbers;
    /// `level=1..5` is the difficulty dial (5, the default, is full strength).
    fn planner(spec: &str) -> Option<Self> {
        let mut parts = spec.split(':');
        if parts.next() != Some("planner") {
            return None;
        }
        let mut params = Params::default();
        let mut tuned = false;
        let mut level = Difficulty::MAX;
        for part in parts {
            let (name, value) = part.split_once('=')?;
            if name == "level" {
                let n: u8 = value.parse().ok()?;
                if !(1..=5).contains(&n) {
                    return None;
                }
                level = Difficulty::new(n);
            } else if params.set(name, value.parse().ok()?) {
                tuned = true;
            } else {
                return None;
            }
        }
        Some(Self::Planner {
            params,
            tuned,
            level,
        })
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Random => "random",
            Self::Heuristic => "heuristic",
            Self::Planner { level, .. } if level.level() < Difficulty::MAX.level() => {
                match level.level() {
                    1 => "planner (novice)",
                    2 => "planner (easy)",
                    3 => "planner (medium)",
                    _ => "planner (hard)",
                }
            },
            Self::Planner { tuned: false, .. } => "planner",
            Self::Planner { tuned: true, .. } => "planner (tuned)",
        }
    }
}

/// Command line options.
#[derive(Debug, PartialEq)]
struct Options {
    game:        Game,
    bot:         BotKind,
    /// One bot per seat, when the seats differ; the seats rotate every game.
    seats:       Vec<BotKind>,
    games:       u64,
    seed:        u64,
    players:     u8,
    max_actions: usize,
    /// Res Arcana's draft variant.
    draft:       bool,
}

const USAGE: &str = "usage: gamelib-cli [--game toy|res-arcana] [--bot \
                     random|heuristic|planner[:name=value..|:level=1..5]] [--bots B1,B2,..] \
                     [--games N] [--seed S] [--players P] [--max-actions M] [--draft]";

fn parse(args: impl IntoIterator<Item = String>) -> Result<Options, String> {
    let mut options = Options {
        game:        Game::Toy,
        bot:         BotKind::Random,
        seats:       Vec::new(),
        games:       1,
        seed:        0,
        players:     2,
        max_actions: 20_000,
        draft:       false,
    };
    let mut args = args.into_iter();
    while let Some(flag) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        let bad = |name: &str| format!("{name} needs a number");
        match flag.as_str() {
            "--game" => {
                options.game = match value("--game")?.as_str() {
                    "toy" => Game::Toy,
                    "res-arcana" => Game::ResArcana,
                    other => return Err(format!("unknown game {other}\n{USAGE}")),
                };
            },
            "--bot" => {
                options.bot = BotKind::parse(&value("--bot")?)?;
            },
            "--bots" => {
                options.seats =
                    value("--bots")?.split(',').map(BotKind::parse).collect::<Result<_, _>>()?;
            },
            "--games" => options.games = value("--games")?.parse().map_err(|_| bad("--games"))?,
            "--seed" => options.seed = value("--seed")?.parse().map_err(|_| bad("--seed"))?,
            "--players" => {
                options.players = value("--players")?.parse().map_err(|_| bad("--players"))?
            },
            "--max-actions" => {
                options.max_actions =
                    value("--max-actions")?.parse().map_err(|_| bad("--max-actions"))?;
            },
            "--draft" => options.draft = true,
            "-h" | "--help" => return Err(USAGE.into()),
            other => return Err(format!("unknown option {other}\n{USAGE}")),
        }
    }
    if !(1..=8).contains(&options.players) {
        return Err("--players must be between 1 and 8".into());
    }
    if !options.seats.is_empty() {
        options.players = u8::try_from(options.seats.len()).unwrap_or(u8::MAX);
    }
    let only_random =
        options.bot == BotKind::Random && options.seats.iter().all(|b| *b == BotKind::Random);
    if !only_random && options.game != Game::ResArcana {
        return Err("the heuristic and planner bots only play res-arcana".into());
    }
    if options.game == Game::ResArcana && !(2..=4).contains(&options.players) {
        return Err("res-arcana is played by 2 to 4 players".into());
    }
    if options.draft && options.game != Game::ResArcana {
        return Err("--draft is a Res Arcana variant (use --game res-arcana)".into());
    }
    Ok(options)
}

/// Plays one game of the chosen kind.
fn play(seed: u64, options: &Options, seats: &[BotKind]) -> Result<Outcome, String> {
    match options.game {
        Game::Toy => play_with(&ToyRules, seed, options, seats),
        Game::ResArcana => play_with(
            &ResArcanaRules::new().with_draft(options.draft),
            seed,
            options,
            seats,
        ),
    }
}

/// The bot of every seat in game number `game`: the listed bots, moved one
/// seat further every game. With different bots every deal (seed) is played
/// once per seating (see [`seed_of`]), so no bot profits from better cards or
/// from starting.
fn seating(options: &Options, game: u64) -> Vec<BotKind> {
    if options.seats.is_empty() {
        return vec![options.bot; usize::from(options.players)];
    }
    let n = options.seats.len();
    let shift = usize::try_from(game % n as u64).unwrap_or(0);
    (0..n).map(|seat| options.seats[(seat + shift) % n]).collect()
}

/// The seed of game number `game`: a new one every game, or with different
/// bots a new one once every seating was played.
fn seed_of(options: &Options, game: u64) -> u64 {
    let seatings = options.seats.len().max(1) as u64;
    options.seed.wrapping_add(game / seatings)
}

/// Plays one game and checks that its log replays to the identical state.
fn play_with<R: GameRules + Clone>(
    rules: &R,
    seed: u64,
    options: &Options,
    seats: &[BotKind],
) -> Result<Outcome, String> {
    let mut engine = Engine::new(rules.clone(), options.players, seed)
        .map_err(|e| format!("setup failed: {e:?}"))?;
    let mut bots: Vec<Box<dyn Bot>> = seats
        .iter()
        .zip(0_u64..)
        .map(|(kind, i)| -> Box<dyn Bot> {
            match kind {
                BotKind::Random => Box::new(RandomBot::new(
                    seed.wrapping_add(i).wrapping_mul(31).wrapping_add(7),
                )),
                BotKind::Heuristic => Box::new(HeuristicBot::with_seed(seed)),
                BotKind::Planner { params, level, .. } => Box::new(PlannerBot::with_difficulty(
                    *params,
                    *level,
                    seed.wrapping_add(i).wrapping_mul(31).wrapping_add(7),
                )),
            }
        })
        .collect();
    let outcome = run(&mut engine, &mut bots, options.max_actions).map_err(|e| match e {
        RunError::Stuck => "nobody can act".to_string(),
        RunError::TooLong(n) => format!("no winner after {n} actions"),
        RunError::Engine(e) => format!("engine rejected a legal action: {e:?}"),
    })?;
    let replay = Engine::replay(rules.clone(), options.players, seed, engine.log())
        .map_err(|e| format!("replay failed: {e:?}"))?;
    if replay.state() != engine.state() {
        return Err("replay diverged from the original game".into());
    }
    Ok(outcome)
}

fn main() -> ExitCode {
    let options = match parse(std::env::args().skip(1)) {
        Ok(o) => o,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        },
    };
    let mut failures = 0_u64;
    let mut total_actions = 0;
    let mut total_rounds = 0_u64;
    let mut wins: std::collections::BTreeMap<&str, u64> = std::collections::BTreeMap::new();
    for i in 0..options.games {
        let seed = seed_of(&options, i);
        let seats = seating(&options, i);
        match play(seed, &options, &seats) {
            Ok(outcome) => {
                total_actions += outcome.actions;
                total_rounds += u64::from(outcome.rounds);
                let winners: Vec<_> = outcome
                    .winners
                    .iter()
                    .map(|p| {
                        let kind = seats.get(usize::from(p.0)).copied().unwrap_or_default();
                        *wins.entry(kind.name()).or_insert(0) += 1;
                        format!("P{} ({})", p.0, kind.name())
                    })
                    .collect();
                println!(
                    "seed {seed}: {} won after {} actions (round {})",
                    winners.join("+"),
                    outcome.actions,
                    outcome.rounds
                );
            },
            Err(message) => {
                failures += 1;
                eprintln!("seed {seed}: FAILED: {message}");
            },
        }
    }
    println!(
        "{} games, {failures} failed, {total_actions} actions",
        options.games
    );
    let played = options.games - failures;
    if played > 0 && options.game == Game::ResArcana {
        #[allow(clippy::cast_precision_loss)]
        let average = total_rounds as f64 / played as f64;
        println!("average length: {average:.2} rounds");
        for (kind, n) in &wins {
            println!("{kind}: {n} wins");
        }
    }
    if failures == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn defaults_and_overrides() {
        assert_eq!(
            parse(args(&[])).unwrap(),
            Options {
                game:        Game::Toy,
                bot:         BotKind::Random,
                seats:       Vec::new(),
                games:       1,
                seed:        0,
                players:     2,
                max_actions: 20_000,
                draft:       false,
            }
        );
        let o = parse(args(&[
            "--games",
            "5",
            "--seed",
            "9",
            "--players",
            "3",
            "--max-actions",
            "100",
        ]))
        .unwrap();
        assert_eq!(
            o,
            Options {
                game:        Game::Toy,
                bot:         BotKind::Random,
                seats:       Vec::new(),
                games:       5,
                seed:        9,
                players:     3,
                max_actions: 100,
                draft:       false,
            }
        );
    }

    #[test]
    fn bad_input_is_explained() {
        assert!(parse(args(&["--games"])).unwrap_err().contains("needs a value"));
        assert!(parse(args(&["--games", "x"])).unwrap_err().contains("needs a number"));
        assert!(parse(args(&["--players", "0"])).unwrap_err().contains("between 1 and 8"));
        assert!(parse(args(&["--wat"])).unwrap_err().contains("unknown option"));
    }

    #[test]
    fn a_whole_game_plays_and_replays() {
        let options = Options {
            game:        Game::Toy,
            bot:         BotKind::Random,
            seats:       Vec::new(),
            games:       1,
            seed:        1,
            players:     2,
            max_actions: 20_000,
            draft:       false,
        };
        let outcome = play(1, &options, &seating(&options, 0)).unwrap();
        assert!(!outcome.winners.is_empty());
    }

    #[test]
    fn the_planner_takes_a_difficulty_level() {
        let level = |spec: &str| match BotKind::parse(spec).unwrap() {
            BotKind::Planner { level, tuned, .. } => (level.level(), tuned),
            other => panic!("not a planner: {other:?}"),
        };
        assert_eq!(level("planner"), (5, false));
        assert_eq!(level("planner:level=2"), (2, false));
        assert_eq!(level("planner:level=3:gold=2"), (3, true));
        assert!(BotKind::parse("planner:level=0").is_err());
        assert!(BotKind::parse("planner:level=6").is_err());
        assert!(BotKind::parse("planner:level=x").is_err());
    }

    #[test]
    fn the_draft_variant_belongs_to_res_arcana() {
        let o = parse(args(&["--game", "res-arcana", "--draft"])).unwrap();
        assert!(o.draft);
        assert!(!parse(args(&["--game", "res-arcana"])).unwrap().draft);
        assert!(parse(args(&["--draft"])).unwrap_err().contains("Res Arcana"));
    }

    #[test]
    fn res_arcana_is_selectable_and_limited_to_two_to_four_players() {
        let o = parse(args(&["--game", "res-arcana", "--players", "3"])).unwrap();
        assert_eq!(o.game, Game::ResArcana);
        assert!(
            parse(args(&["--game", "res-arcana", "--players", "5"]))
                .unwrap_err()
                .contains("2 to 4")
        );
        assert!(parse(args(&["--game", "chess"])).unwrap_err().contains("unknown game"));
    }

    #[test]
    fn a_whole_res_arcana_game_plays_and_replays() {
        let options = Options {
            game:        Game::ResArcana,
            bot:         BotKind::Heuristic,
            seats:       Vec::new(),
            games:       1,
            seed:        1,
            players:     2,
            max_actions: 50_000,
            draft:       false,
        };
        let outcome = play(1, &options, &seating(&options, 0)).unwrap();
        assert!(!outcome.winners.is_empty());
    }
}
