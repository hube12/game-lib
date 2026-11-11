//! What one player is allowed to know about the game.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    counter::Counter,
    effect::Effect,
    id::{DefId, EntityId, PhaseId, PlayerId, ZoneId, ZoneKind},
    modifier::Modifier,
    resource::Pool,
    state::GameState,
    zone::{Order, Owner, Visibility, Zone},
};

/// A card as one player sees it. `def` is `None` when the card is face down to
/// that player.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityView {
    /// Instance id.
    pub id:        EntityId,
    /// Definition, if the viewer may know it.
    pub def:       Option<DefId>,
    /// Face up or down.
    pub face_up:   bool,
    /// Exhausted or ready.
    pub exhausted: bool,
    /// Resources on the card.
    pub pool:      Pool,
}

/// A zone as one player sees it: the count is always known, the entities only
/// when the viewer may see them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZoneView {
    /// Zone instance id.
    pub id:         ZoneId,
    /// Role of the zone.
    pub kind:       ZoneKind,
    /// Owner.
    pub owner:      Owner,
    /// Arrangement.
    pub order:      Order,
    /// Who may see the contents.
    pub visibility: Visibility,
    /// Number of entities, visible or not.
    pub count:      usize,
    /// The entities, empty when the zone is closed to the viewer.
    pub entities:   Vec<EntityView>,
}

/// The decision the game waits for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingView {
    /// Players that still have to answer.
    pub waiting_on: Vec<PlayerId>,
    /// The options, only for a viewer that still has to answer (other players'
    /// choices stay secret).
    pub options:    Option<Vec<Effect>>,
}

/// Everything one player may know, ready to be sent to a client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerView {
    /// Whose view this is.
    pub viewer:    PlayerId,
    /// Player whose turn it is.
    pub active:    Option<PlayerId>,
    /// Current round.
    pub round:     u32,
    /// Running phase.
    pub phase:     Option<PhaseId>,
    /// All zones.
    pub zones:     Vec<ZoneView>,
    /// The shared supply.
    pub bank:      Pool,
    /// Resource pools of all players (public).
    pub pools:     BTreeMap<PlayerId, Pool>,
    /// All counters (public).
    pub counters:  Vec<Counter>,
    /// Live modifiers (public).
    pub modifiers: Vec<Modifier>,
    /// The pending decision, if any.
    pub pending:   Option<PendingView>,
    /// Winners once the game is over.
    pub winners:   Option<Vec<PlayerId>>,
}

fn zone_view(state: &GameState, zone: &Zone, viewer: PlayerId) -> ZoneView {
    let mine = zone.owner.contains(viewer);
    let open = match zone.visibility {
        Visibility::Public => true,
        Visibility::Owner => mine,
        Visibility::Hidden => false,
    };
    let entities = if open {
        zone.entities()
            .iter()
            .filter_map(|id| state.entity(*id).ok())
            .map(|e| EntityView {
                id:        e.id,
                def:       (e.face_up || mine).then_some(e.def),
                face_up:   e.face_up,
                exhausted: e.exhausted,
                pool:      e.pool.clone(),
            })
            .collect()
    } else {
        Vec::new()
    };
    ZoneView {
        id: zone.id,
        kind: zone.kind,
        owner: zone.owner.clone(),
        order: zone.order,
        visibility: zone.visibility,
        count: zone.len(),
        entities,
    }
}

impl PlayerView {
    /// Builds the view of `viewer`.
    pub fn new(state: &GameState, viewer: PlayerId) -> Self {
        let flow = &state.flow;
        Self {
            viewer,
            active: flow.active_player(),
            round: flow.round,
            phase: flow.phase_id(),
            zones: state.zones().map(|z| zone_view(state, z, viewer)).collect(),
            bank: state.bank.clone(),
            pools: state.pools.clone(),
            counters: state.counters.clone(),
            modifiers: state.modifiers.clone(),
            pending: state.pending.as_ref().map(|p| PendingView {
                waiting_on: p.players.iter().copied().filter(|q| p.waiting_for(*q)).collect(),
                options:    p.waiting_for(viewer).then(|| p.options.clone()),
            }),
            winners: state.winners.clone(),
        }
    }
}
