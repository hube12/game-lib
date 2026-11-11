//! Static card definitions and per-instance entity state.

use serde::{Deserialize, Serialize};

use crate::{
    cost::Cost,
    effect::{Ability, Effect},
    id::{DefId, EntityId, ResourceId, ZoneId},
    resource::Pool,
};

/// A placement discount a card grants its owner while it is in play.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Discount {
    /// Only for cards carrying this tag (`None` = every card).
    pub tag:     Option<String>,
    /// How many units of the cost it removes.
    pub n:       u32,
    /// Resource kinds it cannot reduce.
    pub exclude: Vec<ResourceId>,
}

/// Static description of a card, shared by all its copies.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardDef {
    /// Definition id.
    pub id:        DefId,
    /// Display name.
    pub name:      String,
    /// Free-form tags (`"unit"`, `"building"`, ...) games can match on.
    pub tags:      Vec<String>,
    /// What it costs to buy from a market (`None` = free).
    pub cost:      Option<Cost>,
    /// Resolves when a player gains the card by buying it.
    pub on_gain:   Option<Effect>,
    /// Abilities, active while the card sits in a zone with `abilities_active`.
    pub abilities: Vec<Ability>,
    /// Placement discounts it grants while in play.
    pub discounts: Vec<Discount>,
}

/// One card instance and its mutable state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entity {
    /// Instance id.
    pub id:        EntityId,
    /// Its definition.
    pub def:       DefId,
    /// The zone it is currently in.
    pub zone:      ZoneId,
    /// Face up (false = face down / other side hidden).
    pub face_up:   bool,
    /// Exhausted (used) rather than ready.
    pub exhausted: bool,
    /// Resources stored on the card (tokens, eggs, cached food, ...).
    pub pool:      Pool,
}
