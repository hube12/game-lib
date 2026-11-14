//! What a game tells the GUI about itself: names, colors and card text. The GUI
//! never hard-codes a game concept; everything it shows comes from here.

use std::collections::BTreeMap;

use gamelib::id::{CounterId, DefId, PhaseId, ResourceId, ZoneKind};
use serde::{Deserialize, Serialize};

/// A kind of resource (essence, coin, ...).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceInfo {
    /// The engine id.
    pub id:     u16,
    /// Display name.
    pub name:   String,
    /// Short symbol used in card text.
    pub symbol: String,
    /// CSS color.
    pub color:  String,
    /// What it is, for the popup of an underlined term.
    #[serde(default)]
    pub help:   String,
}

/// A role a zone plays.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamedId {
    /// The engine id.
    pub id:   u16,
    /// Display name.
    pub name: String,
    /// What it means, for the popup of an underlined term.
    #[serde(default)]
    pub help: String,
}

/// A card definition as the player reads it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardInfo {
    /// The engine definition id.
    pub id:     u32,
    /// Display name.
    pub name:   String,
    /// The name in another language, when there is one.
    pub alt:    Option<String>,
    /// Free-form tags (family, types).
    pub tags:   Vec<String>,
    /// Placement cost as printed (empty when there is none).
    pub cost:   String,
    /// Victory points as printed (empty when there are none).
    pub points: String,
    /// Printed abilities, one per line.
    pub lines:  Vec<String>,
    /// What the abilities do, in English, one per line, with marked terms (see
    /// `describe::plain`); shown when the card is hovered.
    #[serde(default)]
    pub powers: Vec<String>,
}

/// Everything the GUI needs to know about a game.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameDescription {
    /// Game title.
    pub title:       String,
    /// Fewest players.
    pub min_players: u8,
    /// Most players.
    pub max_players: u8,
    /// Resource kinds.
    pub resources:   Vec<ResourceInfo>,
    /// Zone roles.
    pub zones:       Vec<NamedId>,
    /// Counters.
    pub counters:    Vec<NamedId>,
    /// Phases of a round.
    pub phases:      Vec<NamedId>,
    /// Card definitions.
    pub cards:       Vec<CardInfo>,
    /// Where a card's picture is, with `{id}` for its definition id (for
    /// example `cards/{id}.jpg`); empty when the game has no pictures. A
    /// missing file just means the page shows the card as text.
    #[serde(default)]
    pub card_images: String,
}

/// Name lookups built from a [`GameDescription`].
#[derive(Clone, Debug, Default)]
pub struct Names {
    resources: BTreeMap<u16, String>,
    zones:     BTreeMap<u16, String>,
    counters:  BTreeMap<u16, String>,
    phases:    BTreeMap<u16, String>,
    cards:     BTreeMap<u32, String>,
}

impl Names {
    /// Builds the lookups.
    pub fn new(description: &GameDescription) -> Self {
        let named = |list: &[NamedId]| list.iter().map(|n| (n.id, n.name.clone())).collect();
        Self {
            resources: description.resources.iter().map(|r| (r.id, r.name.clone())).collect(),
            zones:     named(&description.zones),
            counters:  named(&description.counters),
            phases:    named(&description.phases),
            cards:     description.cards.iter().map(|c| (c.id, c.name.clone())).collect(),
        }
    }

    /// Name of a resource.
    pub fn resource(&self, id: ResourceId) -> String {
        self.resources.get(&id.0).cloned().unwrap_or_else(|| format!("resource {}", id.0))
    }

    /// Name of a zone role.
    pub fn zone(&self, kind: ZoneKind) -> String {
        self.zones.get(&kind.0).cloned().unwrap_or_else(|| format!("zone {}", kind.0))
    }

    /// Name of a counter.
    pub fn counter(&self, id: CounterId) -> String {
        self.counters.get(&id.0).cloned().unwrap_or_else(|| format!("counter {}", id.0))
    }

    /// Name of a phase.
    pub fn phase(&self, id: PhaseId) -> String {
        self.phases.get(&id.0).cloned().unwrap_or_else(|| format!("phase {}", id.0))
    }

    /// Name of a card definition.
    pub fn card(&self, id: DefId) -> String {
        self.cards.get(&id.0).cloned().unwrap_or_else(|| format!("card {}", id.0))
    }
}

/// One line of the scoreboard.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScoreLine {
    /// The player.
    pub player: u8,
    /// What is counted ("Victory points").
    pub label:  String,
    /// The value.
    pub value:  i64,
}
