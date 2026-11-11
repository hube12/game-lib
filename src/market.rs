//! Shared zones players buy from, with a policy for which cards are buyable and
//! when the market refills.

use serde::{Deserialize, Serialize};

use crate::{
    id::{EntityId, PlayerId, ZoneId, ZoneKind},
    state::{GameState, StateError},
    zone::{Owner, Pos, Zone},
};

/// When an emptied slot is refilled from the source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Refill {
    /// Right after a card leaves the market.
    Immediate,
    /// At the end of every turn (the engine calls [`Market::refill`]).
    EndOfTurn,
    /// Only when the game asks for it.
    Manual,
}

/// Rules of a market.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketPolicy {
    /// Only the top entity of the market zone may be bought (stack markets).
    pub top_only:       bool,
    /// When slots are refilled.
    pub refill:         Refill,
    /// Where cards removed without being bought go (e.g. a shared discard).
    pub removal:        Option<ZoneId>,
    /// The top card of the source zone may be bought too, without being in the
    /// market first (a blind buy from the deck).
    pub source_buyable: bool,
}

/// Why a market operation failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MarketError {
    /// The entity is not for sale right now.
    NotAvailable(EntityId),
    /// A card must be removed but the policy has no removal zone.
    NoRemovalZone,
    /// The underlying state operation failed.
    State(StateError),
}

impl From<StateError> for MarketError {
    fn from(e: StateError) -> Self {
        Self::State(e)
    }
}

/// A market zone, optionally refilled from a source zone up to `size` cards.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Market {
    /// The zone holding the cards for sale.
    pub zone:   ZoneId,
    /// Where refills come from (top card first).
    pub source: Option<ZoneId>,
    /// Number of cards the market holds when full.
    pub size:   usize,
    /// Kind of the buyer's zone that receives bought cards.
    pub dest:   ZoneKind,
    /// Rules.
    pub policy: MarketPolicy,
}

impl Market {
    /// Whether the entity may be bought now (cost is checked by the engine, not
    /// here).
    pub fn can_buy(&self, state: &GameState, entity: EntityId) -> bool {
        self.offered(state).contains(&entity)
    }

    /// Every entity that may be bought now: the market's cards (just the top
    /// one for stack markets) and, when the policy allows it, the top of the
    /// source.
    pub fn offered(&self, state: &GameState) -> Vec<EntityId> {
        let Ok(zone) = state.zone(self.zone) else {
            return Vec::new();
        };
        let mut out = if self.policy.top_only {
            zone.top().into_iter().collect()
        } else {
            zone.entities().to_vec()
        };
        if self.policy.source_buyable
            && let Some(top) = self.source.and_then(|s| state.zone(s).ok()).and_then(Zone::top)
        {
            out.push(top);
        }
        out
    }

    /// Whether the entity sits in this market or in its source.
    pub fn holds(&self, state: &GameState, entity: EntityId) -> bool {
        state.entity(entity).is_ok_and(|e| e.zone == self.zone || Some(e.zone) == self.source)
    }

    /// Moves a purchasable entity to the buyer and refills when the policy says
    /// so. Paying the cost is the caller's job.
    pub fn buy(
        &self,
        state: &mut GameState,
        player: PlayerId,
        entity: EntityId,
    ) -> Result<(), MarketError> {
        if !self.can_buy(state, entity) {
            return Err(MarketError::NotAvailable(entity));
        }
        let to = state.zone_id(&Owner::Player(player), self.dest)?;
        state.move_entity(entity, to, Pos::Top)?;
        if self.policy.refill == Refill::Immediate {
            self.refill(state)?;
        }
        Ok(())
    }

    /// Removes a card without buying it, sending it to the removal zone.
    pub fn remove(&self, state: &mut GameState, entity: EntityId) -> Result<(), MarketError> {
        if !state.zone(self.zone)?.contains(entity) {
            return Err(MarketError::NotAvailable(entity));
        }
        let to = self.policy.removal.ok_or(MarketError::NoRemovalZone)?;
        state.move_entity(entity, to, Pos::Top)?;
        if self.policy.refill == Refill::Immediate {
            self.refill(state)?;
        }
        Ok(())
    }

    /// Fills the market up to `size` from the source. Returns how many cards
    /// were added.
    pub fn refill(&self, state: &mut GameState) -> Result<usize, StateError> {
        let Some(source) = self.source else {
            return Ok(0);
        };
        let mut added = 0;
        while state.zone(self.zone)?.len() < self.size {
            let Some(top) = state.zone(source)?.top() else {
                break;
            };
            state.move_entity(top, self.zone, Pos::Top)?;
            state.entity_mut(top)?.face_up = true;
            added += 1;
        }
        Ok(added)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        card::CardDef,
        id::DefId,
        rng::Rng,
        zone::{Order, Visibility},
    };

    const HAND: ZoneKind = ZoneKind(1);
    const P0: PlayerId = PlayerId(0);

    struct Fixture {
        state:  GameState,
        market: Market,
        row:    ZoneId,
        supply: ZoneId,
        hand:   ZoneId,
        trash:  ZoneId,
    }

    fn fixture(policy: MarketPolicy, supply_cards: usize) -> Fixture {
        let mut state = GameState::new(Rng::from_seed(1));
        state.add_def(CardDef {
            id:        DefId(0),
            name:      "c".into(),
            tags:      vec![],
            cost:      None,
            on_gain:   None,
            abilities: vec![],
            discounts: vec![],
        });
        state.add_player(P0);
        let row = state.add_zone(ZoneKind(10), Owner::Shared, Order::Row, Visibility::Public);
        let supply = state.add_zone(
            ZoneKind(11),
            Owner::Shared,
            Order::Stack,
            Visibility::Hidden,
        );
        let trash = state.add_zone(
            ZoneKind(12),
            Owner::Shared,
            Order::Stack,
            Visibility::Public,
        );
        let hand = state.add_zone(HAND, Owner::Player(P0), Order::Unordered, Visibility::Owner);
        for _ in 0..supply_cards {
            state.create_entity(DefId(0), supply, false).unwrap();
        }
        let market = Market {
            zone:   row,
            source: Some(supply),
            size:   3,
            dest:   HAND,
            policy: MarketPolicy {
                removal: Some(trash),
                source_buyable: false,
                ..policy
            },
        };
        market.refill(&mut state).unwrap();
        Fixture {
            state,
            market,
            row,
            supply,
            hand,
            trash,
        }
    }

    const ROW_POLICY: MarketPolicy = MarketPolicy {
        top_only:       false,
        refill:         Refill::Immediate,
        removal:        None,
        source_buyable: false,
    };

    #[test]
    fn initial_refill_fills_the_row_face_up() {
        let f = fixture(ROW_POLICY, 5);
        assert_eq!(f.state.zone(f.row).unwrap().len(), 3);
        assert_eq!(f.state.zone(f.supply).unwrap().len(), 2);
        assert!(
            f.state.zone(f.row).unwrap().entities().iter().all(|e| f
                .state
                .entity(*e)
                .unwrap()
                .face_up)
        );
    }

    #[test]
    fn buying_moves_the_card_and_refills_immediately() {
        let mut f = fixture(ROW_POLICY, 5);
        let card = f.state.zone(f.row).unwrap().entities()[1];
        f.market.buy(&mut f.state, P0, card).unwrap();
        assert!(f.state.zone(f.hand).unwrap().contains(card));
        assert_eq!(f.state.zone(f.row).unwrap().len(), 3);
        assert_eq!(f.state.zone(f.supply).unwrap().len(), 1);
    }

    #[test]
    fn end_of_turn_refill_waits_for_the_call() {
        let mut f = fixture(
            MarketPolicy {
                refill: Refill::EndOfTurn,
                ..ROW_POLICY
            },
            5,
        );
        let card = f.state.zone(f.row).unwrap().entities()[0];
        f.market.buy(&mut f.state, P0, card).unwrap();
        assert_eq!(f.state.zone(f.row).unwrap().len(), 2);
        assert_eq!(f.market.refill(&mut f.state).unwrap(), 1);
        assert_eq!(f.state.zone(f.row).unwrap().len(), 3);
    }

    #[test]
    fn top_only_allows_just_the_top_card() {
        let mut f = fixture(
            MarketPolicy {
                top_only: true,
                ..ROW_POLICY
            },
            5,
        );
        let entities = f.state.zone(f.row).unwrap().entities().to_vec();
        assert!(f.market.can_buy(&f.state, entities[2]));
        assert!(!f.market.can_buy(&f.state, entities[0]));
        assert_eq!(
            f.market.buy(&mut f.state, P0, entities[0]),
            Err(MarketError::NotAvailable(entities[0]))
        );
        assert!(f.state.zone(f.hand).unwrap().is_empty());
    }

    #[test]
    fn short_supply_leaves_the_market_short() {
        let f = fixture(ROW_POLICY, 2);
        assert_eq!(f.state.zone(f.row).unwrap().len(), 2);
        assert!(f.state.zone(f.supply).unwrap().is_empty());
    }

    #[test]
    fn removal_goes_to_the_removal_zone_and_refills() {
        let mut f = fixture(ROW_POLICY, 5);
        let card = f.state.zone(f.row).unwrap().entities()[0];
        f.market.remove(&mut f.state, card).unwrap();
        assert!(f.state.zone(f.trash).unwrap().contains(card));
        assert_eq!(f.state.zone(f.row).unwrap().len(), 3);
    }

    #[test]
    fn removal_without_a_removal_zone_is_an_error() {
        let mut f = fixture(ROW_POLICY, 5);
        f.market.policy.removal = None;
        let card = f.state.zone(f.row).unwrap().entities()[0];
        assert_eq!(
            f.market.remove(&mut f.state, card),
            Err(MarketError::NoRemovalZone)
        );
        assert!(f.state.zone(f.row).unwrap().contains(card));
    }
}
