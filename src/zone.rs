//! Zones: generic containers for entities (deck, hand, discard, market row,
//! tableau, ...).

use serde::{Deserialize, Serialize};

use crate::id::{EntityId, PlayerId, ZoneId, ZoneKind};

/// Who a zone (or counter) belongs to.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Owner {
    /// Shared by everyone.
    Shared,
    /// Belongs to one player.
    Player(PlayerId),
    /// Belongs to a team. Lookups compare the list as written, so keep it in a
    /// fixed (ascending) order.
    Players(Vec<PlayerId>),
}

impl Owner {
    /// Whether the player owns it alone or as a member of the owning team.
    /// Shared things belong to nobody.
    pub fn contains(&self, player: PlayerId) -> bool {
        match self {
            Self::Shared => false,
            Self::Player(p) => *p == player,
            Self::Players(team) => team.contains(&player),
        }
    }

    /// The player that stands for the owner when one must be picked: the player
    /// itself, or the first team member.
    pub fn lead(&self) -> Option<PlayerId> {
        match self {
            Self::Shared => None,
            Self::Player(p) => Some(*p),
            Self::Players(team) => team.first().copied(),
        }
    }
}

/// How the entities of a zone are arranged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Order {
    /// Last entity is the top; only the top is normally reachable (decks,
    /// discard piles).
    Stack,
    /// Ordered left to right (market rows).
    Row,
    /// No meaningful order (hands, tableaus).
    Unordered,
}

/// Who can see the contents.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Visibility {
    /// Everyone sees every entity.
    Public,
    /// Only the owner sees the entities; others only see how many there are.
    Owner,
    /// Nobody sees the entities, only the count.
    Hidden,
}

/// Where to insert an entity in a zone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Pos {
    /// Top of a stack / right end of a row.
    Top,
    /// Bottom of a stack / left end of a row.
    Bottom,
    /// At an explicit index (`0..=len`).
    Index(usize),
}

/// A container of entities.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Zone {
    /// Zone instance id.
    pub id:               ZoneId,
    /// Role of the zone (per owner there is at most one zone per kind).
    pub kind:             ZoneKind,
    /// Owner of the zone.
    pub owner:            Owner,
    /// Arrangement of the entities.
    pub order:            Order,
    /// Who can see the entities.
    pub visibility:       Visibility,
    /// Whether abilities of entities in this zone are active (tableaus yes,
    /// decks no).
    pub abilities_active: bool,
    /// When drawing from an empty zone, reshuffle the zone of this kind (same
    /// owner) into it.
    pub recycle_from:     Option<ZoneKind>,
    entities:             Vec<EntityId>,
}

impl Zone {
    /// Creates an empty zone with no active abilities and no recycling.
    pub const fn new(
        id: ZoneId,
        kind: ZoneKind,
        owner: Owner,
        order: Order,
        visibility: Visibility,
    ) -> Self {
        Self {
            id,
            kind,
            owner,
            order,
            visibility,
            abilities_active: false,
            recycle_from: None,
            entities: Vec::new(),
        }
    }

    /// Entities from bottom/left to top/right.
    pub fn entities(&self) -> &[EntityId] {
        &self.entities
    }

    /// Number of entities.
    pub const fn len(&self) -> usize {
        self.entities.len()
    }

    /// Whether the zone is empty.
    pub const fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    /// The top entity, if any.
    pub fn top(&self) -> Option<EntityId> {
        self.entities.last().copied()
    }

    /// Whether the entity is in this zone.
    pub fn contains(&self, entity: EntityId) -> bool {
        self.entities.contains(&entity)
    }

    pub(crate) const fn entities_mut(&mut self) -> &mut Vec<EntityId> {
        &mut self.entities
    }

    /// Inserts at a position; returns `false` when an explicit index is out of
    /// range.
    pub(crate) fn insert(&mut self, entity: EntityId, pos: Pos) -> bool {
        match pos {
            Pos::Top => self.entities.push(entity),
            Pos::Bottom => self.entities.insert(0, entity),
            Pos::Index(i) if i <= self.entities.len() => self.entities.insert(i, entity),
            Pos::Index(_) => return false,
        }
        true
    }

    /// Removes an entity; returns `false` when it was not here.
    pub(crate) fn remove(&mut self, entity: EntityId) -> bool {
        self.entities.iter().position(|e| *e == entity).map(|i| self.entities.remove(i)).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zone() -> Zone {
        Zone::new(
            ZoneId(0),
            ZoneKind(0),
            Owner::Shared,
            Order::Row,
            Visibility::Public,
        )
    }

    #[test]
    fn insert_positions() {
        let mut z = zone();
        assert!(z.insert(EntityId(1), Pos::Top));
        assert!(z.insert(EntityId(2), Pos::Bottom));
        assert!(z.insert(EntityId(3), Pos::Index(1)));
        assert_eq!(z.entities(), &[EntityId(2), EntityId(3), EntityId(1)]);
        assert_eq!(z.top(), Some(EntityId(1)));
        assert!(!z.insert(EntityId(4), Pos::Index(9)));
        assert_eq!(z.len(), 3);
    }

    #[test]
    fn owner_membership_covers_players_and_teams() {
        let team = Owner::Players(vec![PlayerId(0), PlayerId(2)]);
        assert!(team.contains(PlayerId(2)) && !team.contains(PlayerId(1)));
        assert!(Owner::Player(PlayerId(1)).contains(PlayerId(1)));
        assert!(!Owner::Shared.contains(PlayerId(0)));
        assert_eq!(team.lead(), Some(PlayerId(0)));
        assert_eq!(Owner::Shared.lead(), None);
    }

    #[test]
    fn remove_reports_presence() {
        let mut z = zone();
        z.insert(EntityId(1), Pos::Top);
        assert!(z.remove(EntityId(1)));
        assert!(!z.remove(EntityId(1)));
        assert!(z.is_empty());
    }
}
