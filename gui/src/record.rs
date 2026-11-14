//! Writes every game to its own file, one JSON object per line, so games can
//! be studied afterwards (how fast somebody won, what a bot was thinking).
//!
//! The lines, in order: one `start`, then an `action` for every move (with the
//! raw engine action, so seed + actions replay the game), a `round` with every
//! player's cards and resources whenever a new round begins, and one `end`.
//! The file holds hidden information (hands, decks): it is a tool for the
//! developer, not something to show a player during the game.

use std::{
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use gamelib::{id::PlayerId, resource::Supply, state::GameState, zone::Owner};
use serde_json::{Map, Value, json};

use crate::description::Names;

/// The calendar date of a day count since 1970-01-01 (proleptic Gregorian).
const fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + if month <= 2 { 1 } else { 0 };
    (year, month, day)
}

/// `2026-11-15_16-30-05` for a number of seconds since 1970 (UTC).
fn stamp(seconds: u64) -> String {
    let seconds = i64::try_from(seconds).unwrap_or(0);
    let (year, month, day) = civil(seconds.div_euclid(86_400));
    let rest = seconds.rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02}_{:02}-{:02}-{:02}",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60
    )
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// The log file of one game.
#[derive(Debug)]
pub struct Recorder {
    path:  PathBuf,
    file:  File,
    /// The round the last `round` line was written for.
    round: Option<u32>,
}

impl Recorder {
    /// Creates `<dir>/<date>_<time>_seed<seed>.jsonl` (the directory too). A
    /// number is appended when the name is taken.
    pub fn create(dir: &Path, seed: u64) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let base = format!("{}_seed{seed}", stamp(now()));
        for attempt in 0..1_000 {
            let name = if attempt == 0 {
                format!("{base}.jsonl")
            } else {
                format!("{base}_{attempt}.jsonl")
            };
            let path = dir.join(name);
            match File::options().write(true).create_new(true).open(&path) {
                Ok(file) => {
                    return Ok(Self {
                        path,
                        file,
                        round: None,
                    });
                },
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {},
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::other("no free log file name"))
    }

    /// Where the game is written.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Appends one line. A full disk must not stop the game, so errors are
    /// dropped.
    pub fn write(&mut self, line: &Value) {
        if let Ok(text) = serde_json::to_string(line) {
            let _ = writeln!(self.file, "{text}");
            let _ = self.file.flush();
        }
    }

    /// Writes a `round` line when the state entered a round not yet written.
    pub fn round(&mut self, state: &GameState, names: &Names, scores: &Value) {
        let round = state.flow.round;
        if self.round == Some(round) {
            return;
        }
        self.round = Some(round);
        self.write(&json!({
            "type": "round",
            "round": round,
            "order": state.flow.turn_order.order().iter().map(|p| p.0).collect::<Vec<_>>(),
            "scores": scores,
            "players": positions(state, names),
            "table": zones_of(state, names, &Owner::Shared),
        }));
    }
}

/// The resources of a pool by name, leaving out what is not held.
fn pool(pool: &gamelib::resource::Pool, names: &Names) -> Value {
    let mut out = Map::new();
    for (res, supply) in pool.iter() {
        if let Supply::Finite(n) = supply
            && n > 0
        {
            out.insert(names.resource(res), json!(n));
        }
    }
    Value::Object(out)
}

/// Every zone of an owner with its cards by name (turned cards and resources
/// on cards are noted).
fn zones_of(state: &GameState, names: &Names, owner: &Owner) -> Value {
    let mut out = Map::new();
    for zone in state.zones().filter(|z| z.owner == *owner) {
        let cards: Vec<Value> = zone
            .entities()
            .iter()
            .filter_map(|id| state.entity(*id).ok())
            .map(|e| {
                let name = names.card(e.def);
                let held = pool(&e.pool, names);
                let empty = held.as_object().is_none_or(Map::is_empty);
                if !e.exhausted && empty {
                    json!(name)
                } else {
                    json!({ "card": name, "turned": e.exhausted, "on": held })
                }
            })
            .collect();
        if !cards.is_empty() {
            out.insert(names.zone(zone.kind), Value::Array(cards));
        }
    }
    Value::Object(out)
}

/// What every player holds: resources and the cards of each of their zones.
pub fn positions(state: &GameState, names: &Names) -> Value {
    let players: Vec<Value> = state
        .flow
        .turn_order
        .order()
        .iter()
        .map(|p| {
            json!({
                "player": p.0,
                "pool": state.player_pool(*p).map_or(Value::Null, |held| pool(held, names)),
                "zones": zones_of(state, names, &Owner::Player(*p)),
            })
        })
        .collect();
    Value::Array(players)
}

/// The resources of every player, by seat.
pub fn pools(state: &GameState, names: &Names) -> Value {
    let mut out = Map::new();
    for p in state.flow.turn_order.order() {
        let held = state.player_pool(*p).map_or(Value::Null, |held| pool(held, names));
        out.insert(p.0.to_string(), held);
    }
    Value::Object(out)
}

/// The seats as numbers.
pub fn seats(players: &[PlayerId]) -> Vec<u8> {
    players.iter().map(|p| p.0).collect()
}

/// The time a log line is written (seconds since 1970).
pub fn time() -> u64 {
    now()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_carry_the_date_and_the_seed() {
        assert_eq!(stamp(0), "1970-01-01_00-00-00");
        assert_eq!(stamp(1_794_760_205), "2026-11-15_16-30-05");
        assert_eq!(stamp(951_782_400), "2000-02-29_00-00-00");
    }

    #[test]
    fn two_games_of_the_same_second_get_their_own_files() {
        let dir = std::env::temp_dir().join(format!("gamelib-gui-record-{}", std::process::id()));
        let mut a = Recorder::create(&dir, 7).unwrap();
        let b = Recorder::create(&dir, 7).unwrap();
        assert_ne!(a.path(), b.path());
        a.write(&json!({ "type": "start" }));
        a.write(&json!({ "type": "end" }));
        let text = fs::read_to_string(a.path()).unwrap();
        assert_eq!(text.lines().count(), 2, "one object per line");
        assert!(a.path().file_name().unwrap().to_str().unwrap().contains("seed7"));
        fs::remove_dir_all(dir).unwrap();
    }
}
