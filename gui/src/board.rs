//! The board as the frontend reads it: plain JSON-friendly shapes that do not
//! depend on the engine's internal types.

use gamelib::{
    id::PlayerId,
    resource::{Pool, Supply},
    view::PlayerView,
    zone::{Owner, Visibility},
};
use serde::{Deserialize, Serialize};

/// An amount of one resource.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResAmount {
    /// The resource id.
    pub res: u16,
    /// How many.
    pub n:   u32,
}

/// A counter value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CounterValue {
    /// The counter id.
    pub counter: u16,
    /// Its value.
    pub value:   i64,
}

/// A card as one player sees it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardDto {
    /// Instance id (what actions refer to).
    pub id:        u32,
    /// Definition id; `None` when the card is face down to the viewer.
    pub def:       Option<u32>,
    /// Face up or down.
    pub face_up:   bool,
    /// Turned (used) or ready.
    pub exhausted: bool,
    /// Resources sitting on the card.
    pub pool:      Vec<ResAmount>,
}

/// A zone as one player sees it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZoneDto {
    /// Zone instance id.
    pub id:         u32,
    /// The zone's role.
    pub kind:       u16,
    /// Owners; `None` means shared.
    pub owner:      Option<Vec<u8>>,
    /// `public`, `owner` or `hidden`.
    pub visibility: String,
    /// How many cards it holds, seen or not.
    pub count:      usize,
    /// The cards the viewer may see.
    pub cards:      Vec<CardDto>,
}

/// One seat.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerDto {
    /// The seat number.
    pub id:       u8,
    /// Display name.
    pub name:     String,
    /// Whether a person plays this seat.
    pub human:    bool,
    /// Whether they passed in the running phase.
    pub passed:   bool,
    /// Their resource pool.
    pub pool:     Vec<ResAmount>,
    /// Their counters.
    pub counters: Vec<CounterValue>,
}

/// Everything on the table that one viewer may know.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardView {
    /// Whose view this is.
    pub viewer:  u8,
    /// Player whose turn it is.
    pub active:  Option<u8>,
    /// Current round.
    pub round:   u32,
    /// Running phase.
    pub phase:   Option<u16>,
    /// Seats in turn order.
    pub players: Vec<PlayerDto>,
    /// Counters nobody owns.
    pub shared:  Vec<CounterValue>,
    /// All zones.
    pub zones:   Vec<ZoneDto>,
    /// Players whose answer a pending decision still waits for.
    pub waiting: Vec<u8>,
}

pub(crate) fn amounts(pool: &Pool) -> Vec<ResAmount> {
    pool.iter()
        .filter_map(|(res, supply)| match supply {
            Supply::Finite(n) if n > 0 => Some(ResAmount { res: res.0, n }),
            _ => None,
        })
        .collect()
}

impl BoardView {
    /// Converts an engine view. `order` is the turn order, `passed` who passed,
    /// `humans` who is a person.
    pub fn new(
        view: &PlayerView,
        order: &[PlayerId],
        passed: &[PlayerId],
        humans: &[PlayerId],
    ) -> Self {
        let players = order
            .iter()
            .map(|p| PlayerDto {
                id:       p.0,
                name:     crate::describe::player_name(*p),
                human:    humans.contains(p),
                passed:   passed.contains(p),
                pool:     view.pools.get(p).map(amounts).unwrap_or_default(),
                counters: view
                    .counters
                    .iter()
                    .filter(|c| c.owner == Owner::Player(*p))
                    .map(|c| CounterValue {
                        counter: c.id.0,
                        value:   c.value,
                    })
                    .collect(),
            })
            .collect();
        let shared = view
            .counters
            .iter()
            .filter(|c| c.owner == Owner::Shared)
            .map(|c| CounterValue {
                counter: c.id.0,
                value:   c.value,
            })
            .collect();
        let zones = view
            .zones
            .iter()
            .map(|z| ZoneDto {
                id:         z.id.0,
                kind:       z.kind.0,
                owner:      match &z.owner {
                    Owner::Shared => None,
                    Owner::Player(p) => Some(vec![p.0]),
                    Owner::Players(team) => Some(team.iter().map(|p| p.0).collect()),
                },
                visibility: match z.visibility {
                    Visibility::Public => "public",
                    Visibility::Owner => "owner",
                    Visibility::Hidden => "hidden",
                }
                .to_owned(),
                count:      z.count,
                cards:      z
                    .entities
                    .iter()
                    .map(|e| CardDto {
                        id:        e.id.0,
                        def:       e.def.map(|d| d.0),
                        face_up:   e.face_up,
                        exhausted: e.exhausted,
                        pool:      amounts(&e.pool),
                    })
                    .collect(),
            })
            .collect();
        Self {
            viewer: view.viewer.0,
            active: view.active.map(|p| p.0),
            round: view.round,
            phase: view.phase.map(|p| p.0),
            players,
            shared,
            zones,
            waiting: view
                .pending
                .as_ref()
                .map(|p| p.waiting_on.iter().map(|q| q.0).collect())
                .unwrap_or_default(),
        }
    }
}
