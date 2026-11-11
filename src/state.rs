//! The complete, serializable game state and the primitive operations on it.

use std::collections::{BTreeMap, VecDeque};

use serde::{Deserialize, Serialize};

use crate::{
    card::{CardDef, Entity},
    counter::Counter,
    effect::{AbilityUse, Effect, PendingDecision, Task},
    event::Event,
    flow::Flow,
    id::{CounterId, DefId, EntityId, PlayerId, ResourceId, ZoneId, ZoneKind},
    market::Market,
    modifier::{Modifier, Subject},
    resource::{Bank, Pool, ResourceError, Supply, transfer},
    rng::Rng,
    zone::{Order, Owner, Pos, Visibility, Zone},
};

/// Why a state operation was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StateError {
    /// Unknown entity.
    NoEntity(EntityId),
    /// Unknown zone.
    NoZone(ZoneId),
    /// The owner has no zone of that kind.
    NoZoneKind(Owner, ZoneKind),
    /// Unknown card definition.
    NoDef(DefId),
    /// Unknown player.
    NoPlayer(PlayerId),
    /// An explicit insertion index was out of range.
    BadPosition,
    /// A resource operation failed.
    Resource(ResourceError),
}

impl From<ResourceError> for StateError {
    fn from(e: ResourceError) -> Self {
        Self::Resource(e)
    }
}

/// Which zone kinds play the roles the engine's card actions need.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardRules {
    /// Where a player's cards wait to be placed or discarded.
    pub hand:       ZoneKind,
    /// Where placed cards go (their abilities are active there).
    pub tableau:    ZoneKind,
    /// Where discarded and destroyed cards go.
    pub discard:    ZoneKind,
    /// Resolves when a card is discarded from hand as an action (for example,
    /// "gain 1 coin or any 2 other resources").
    pub on_discard: Option<Effect>,
}

/// Everything that makes up one game in progress.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameState {
    /// Card definitions by id.
    pub defs:       BTreeMap<DefId, CardDef>,
    entities:       BTreeMap<EntityId, Entity>,
    zones:          BTreeMap<ZoneId, Zone>,
    /// The shared supply.
    pub bank:       Bank,
    /// Resource pool of each player.
    pub pools:      BTreeMap<PlayerId, Pool>,
    /// All counters.
    pub counters:   Vec<Counter>,
    /// Markets players can buy from.
    pub markets:    Vec<Market>,
    /// Phases, turn order and round bookkeeping.
    pub flow:       Flow,
    /// Zone roles for the place and discard actions, if the game has them.
    pub card_rules: Option<CardRules>,
    /// Live rule changes.
    pub modifiers:  Vec<Modifier>,
    /// Limited abilities that were used and have not reset yet.
    pub used:       Vec<AbilityUse>,
    /// Work waiting to be resolved (last = next).
    pub stack:      Vec<Task>,
    /// The decision the game waits for, if any.
    pub pending:    Option<PendingDecision>,
    /// Set once the game is over.
    pub winners:    Option<Vec<PlayerId>>,
    events:         VecDeque<Event>,
    /// The game's random source.
    pub rng:        Rng,
    next_entity:    u32,
    next_zone:      u32,
}

impl GameState {
    /// An empty state driven by the given generator.
    pub fn new(rng: Rng) -> Self {
        Self {
            defs: BTreeMap::new(),
            entities: BTreeMap::new(),
            zones: BTreeMap::new(),
            bank: Bank::new(),
            pools: BTreeMap::new(),
            counters: Vec::new(),
            markets: Vec::new(),
            flow: Flow::default(),
            card_rules: None,
            modifiers: Vec::new(),
            used: Vec::new(),
            stack: Vec::new(),
            pending: None,
            winners: None,
            events: VecDeque::new(),
            rng,
            next_entity: 0,
            next_zone: 0,
        }
    }

    /// Queues an event; the engine processes queued events in order after each
    /// step.
    pub fn emit(&mut self, event: Event) {
        self.events.push_back(event);
    }

    /// Takes the oldest queued event.
    pub fn pop_event(&mut self) -> Option<Event> {
        self.events.pop_front()
    }

    /// Forgets queued events (used after setup so preparing the table triggers
    /// nothing).
    pub fn clear_events(&mut self) {
        self.events.clear();
    }

    /// Registers a card definition.
    pub fn add_def(&mut self, def: CardDef) {
        self.defs.insert(def.id, def);
    }

    /// Gives a player an empty resource pool.
    pub fn add_player(&mut self, player: PlayerId) {
        self.pools.entry(player).or_default();
    }

    /// Adds an empty zone and returns its id.
    pub fn add_zone(
        &mut self,
        kind: ZoneKind,
        owner: Owner,
        order: Order,
        visibility: Visibility,
    ) -> ZoneId {
        let id = ZoneId(self.next_zone);
        self.next_zone += 1;
        self.zones.insert(id, Zone::new(id, kind, owner, order, visibility));
        id
    }

    /// Adds a counter.
    pub fn add_counter(&mut self, counter: Counter) {
        self.counters.push(counter);
    }

    /// A zone by id.
    pub fn zone(&self, id: ZoneId) -> Result<&Zone, StateError> {
        self.zones.get(&id).ok_or(StateError::NoZone(id))
    }

    /// A mutable zone by id (to set flags such as `abilities_active`).
    pub fn zone_mut(&mut self, id: ZoneId) -> Result<&mut Zone, StateError> {
        self.zones.get_mut(&id).ok_or(StateError::NoZone(id))
    }

    /// All zones in id order.
    pub fn zones(&self) -> impl Iterator<Item = &Zone> {
        self.zones.values()
    }

    /// The zone of a given kind owned by `owner`.
    pub fn zone_id(&self, owner: &Owner, kind: ZoneKind) -> Result<ZoneId, StateError> {
        self.zones
            .values()
            .find(|z| z.owner == *owner && z.kind == kind)
            .map(|z| z.id)
            .ok_or_else(|| StateError::NoZoneKind(owner.clone(), kind))
    }

    /// An entity by id.
    pub fn entity(&self, id: EntityId) -> Result<&Entity, StateError> {
        self.entities.get(&id).ok_or(StateError::NoEntity(id))
    }

    /// A mutable entity by id.
    pub fn entity_mut(&mut self, id: EntityId) -> Result<&mut Entity, StateError> {
        self.entities.get_mut(&id).ok_or(StateError::NoEntity(id))
    }

    /// All entities in id order.
    pub fn entities(&self) -> impl Iterator<Item = &Entity> {
        self.entities.values()
    }

    /// Creates an entity of a known definition on top of a zone.
    pub fn create_entity(
        &mut self,
        def: DefId,
        zone: ZoneId,
        face_up: bool,
    ) -> Result<EntityId, StateError> {
        if !self.defs.contains_key(&def) {
            return Err(StateError::NoDef(def));
        }
        let id = EntityId(self.next_entity);
        self.zone_mut(zone)?.insert(id, Pos::Top);
        self.next_entity += 1;
        self.entities.insert(
            id,
            Entity {
                id,
                def,
                zone,
                face_up,
                exhausted: false,
                pool: Pool::new(),
            },
        );
        Ok(id)
    }

    /// Moves an entity to another zone. The single way entities change zone.
    pub fn move_entity(
        &mut self,
        entity: EntityId,
        to: ZoneId,
        pos: Pos,
    ) -> Result<(), StateError> {
        let from = self.entity(entity)?.zone;
        self.zone(to)?;
        // Validate the position before touching anything so a failure leaves
        // the state intact.
        if let Pos::Index(i) = pos {
            let len = self.zone(to)?.len() - usize::from(from == to);
            if i > len {
                return Err(StateError::BadPosition);
            }
        }
        self.zone_mut(from)?.remove(entity);
        self.zone_mut(to)?.insert(entity, pos);
        self.entity_mut(entity)?.zone = to;
        self.emit(Event::EntityMoved { entity, from, to });
        Ok(())
    }

    /// Shuffles a zone with the game's generator.
    pub fn shuffle(&mut self, zone: ZoneId) -> Result<(), StateError> {
        let mut entities = std::mem::take(self.zone_mut(zone)?.entities_mut());
        self.rng.shuffle(&mut entities);
        *self.zone_mut(zone)?.entities_mut() = entities;
        self.emit(Event::Shuffled(zone));
        Ok(())
    }

    /// Reorders the top `perm.len()` cards of a zone: the card that ends up
    /// `i`-th from the top is the one that was `perm[i]`-th from the top.
    pub fn reorder_top(&mut self, zone: ZoneId, perm: &[usize]) -> Result<(), StateError> {
        let n = perm.len();
        let mut sorted = perm.to_vec();
        sorted.sort_unstable();
        if sorted.iter().enumerate().any(|(i, p)| i != *p) || n > self.zone(zone)?.len() {
            return Err(StateError::BadPosition);
        }
        let entities = self.zone_mut(zone)?.entities_mut();
        let len = entities.len();
        let top_first: Vec<_> = entities.iter().rev().take(n).copied().collect();
        for (i, from) in perm.iter().enumerate() {
            entities[len - 1 - i] = top_first[*from];
        }
        self.emit(Event::Shuffled(zone));
        Ok(())
    }

    /// Draws up to `n` entities from the `from` zone to the `to` zone of
    /// `owner`.
    ///
    /// When the source runs out and has `recycle_from` set, that zone is
    /// reshuffled into it and drawing goes on. Returns how many were really
    /// drawn.
    pub fn draw(
        &mut self,
        owner: &Owner,
        from: ZoneKind,
        to: ZoneKind,
        n: u32,
    ) -> Result<u32, StateError> {
        let from = self.zone_id(owner, from)?;
        let to = self.zone_id(owner, to)?;
        let mut drawn = 0;
        for _ in 0..n {
            if self.zone(from)?.is_empty() {
                self.recycle(from)?;
            }
            let Some(top) = self.zone(from)?.top() else {
                break;
            };
            self.move_entity(top, to, Pos::Top)?;
            drawn += 1;
        }
        Ok(drawn)
    }

    /// Makes sure `deck` holds at least `n` cards to look at, when it has a
    /// recycle zone: the cards still in it stay on top, and the recycle pile,
    /// shuffled, goes underneath. Nothing happens when the deck is deep enough.
    pub fn top_up(&mut self, deck: ZoneId, n: usize) -> Result<(), StateError> {
        let zone = self.zone(deck)?;
        if zone.len() >= n {
            return Ok(());
        }
        let Some(kind) = zone.recycle_from else {
            return Ok(());
        };
        let source = self.zone_id(&zone.owner.clone(), kind)?;
        let mut pile = self.zone(source)?.entities().to_vec();
        if pile.is_empty() {
            return Ok(());
        }
        self.rng.shuffle(&mut pile);
        for e in pile {
            self.move_entity(e, deck, Pos::Bottom)?;
        }
        self.emit(Event::Shuffled(deck));
        Ok(())
    }

    fn recycle(&mut self, deck: ZoneId) -> Result<(), StateError> {
        let zone = self.zone(deck)?;
        let Some(kind) = zone.recycle_from else {
            return Ok(());
        };
        let source = self.zone_id(&zone.owner.clone(), kind)?;
        for e in self.zone(source)?.entities().to_vec() {
            self.move_entity(e, deck, Pos::Top)?;
        }
        self.shuffle(deck)
    }

    /// The pool of a player.
    pub fn player_pool(&self, player: PlayerId) -> Result<&Pool, StateError> {
        self.pools.get(&player).ok_or(StateError::NoPlayer(player))
    }

    /// Moves resources from the bank to a player, capped to what the bank has.
    /// Returns the amount moved.
    pub fn gain_player(
        &mut self,
        player: PlayerId,
        res: ResourceId,
        n: u32,
    ) -> Result<u32, StateError> {
        let pool = self.pools.get_mut(&player).ok_or(StateError::NoPlayer(player))?;
        let n = self.bank.finite(res).map_or(n, |have| have.min(n));
        transfer(&mut self.bank, pool, res, n)?;
        if n > 0 {
            self.emit(Event::ResourceGained {
                to: Subject::Player(player),
                res,
                n,
            });
        }
        Ok(n)
    }

    /// Moves resources from the bank to an entity, capped to what the bank has.
    /// Returns the amount moved.
    pub fn gain_entity(
        &mut self,
        entity: EntityId,
        res: ResourceId,
        n: u32,
    ) -> Result<u32, StateError> {
        let n = self.bank.finite(res).map_or(n, |have| have.min(n));
        let pool = &mut self.entities.get_mut(&entity).ok_or(StateError::NoEntity(entity))?.pool;
        transfer(&mut self.bank, pool, res, n)?;
        if n > 0 {
            self.emit(Event::ResourceGained {
                to: Subject::Entity(entity),
                res,
                n,
            });
        }
        Ok(n)
    }

    /// Moves exactly `n` resources from a player back to the bank.
    pub fn pay_player(
        &mut self,
        player: PlayerId,
        res: ResourceId,
        n: u32,
    ) -> Result<(), StateError> {
        let pool = self.pools.get_mut(&player).ok_or(StateError::NoPlayer(player))?;
        transfer(pool, &mut self.bank, res, n)?;
        if n > 0 {
            self.emit(Event::ResourcePaid { player, res, n });
        }
        Ok(())
    }

    /// Moves every resource on a card into a player's pool.
    pub fn take_all(&mut self, entity: EntityId, player: PlayerId) -> Result<(), StateError> {
        let held: Vec<_> = self.entity(entity)?.pool.iter().collect();
        let pool = self.pools.get_mut(&player).ok_or(StateError::NoPlayer(player))?;
        let card = &mut self.entities.get_mut(&entity).ok_or(StateError::NoEntity(entity))?.pool;
        for (res, supply) in held {
            if let Supply::Finite(n) = supply
                && n > 0
            {
                transfer(card, pool, res, n)?;
                self.events.push_back(Event::ResourceGained {
                    to: Subject::Player(player),
                    res,
                    n,
                });
            }
        }
        Ok(())
    }

    /// Returns every resource on a card to the bank.
    pub fn return_to_bank(&mut self, entity: EntityId) -> Result<(), StateError> {
        let held: Vec<_> = self.entity(entity)?.pool.iter().collect();
        let card = &mut self.entities.get_mut(&entity).ok_or(StateError::NoEntity(entity))?.pool;
        for (res, supply) in held {
            if let Supply::Finite(n) = supply
                && n > 0
            {
                transfer(card, &mut self.bank, res, n)?;
            }
        }
        Ok(())
    }

    /// Takes `n` of a resource off a card back to the bank.
    pub fn take_from_card(
        &mut self,
        entity: EntityId,
        res: ResourceId,
        n: u32,
    ) -> Result<(), StateError> {
        let card = &mut self.entities.get_mut(&entity).ok_or(StateError::NoEntity(entity))?.pool;
        transfer(card, &mut self.bank, res, n)?;
        Ok(())
    }

    /// Adds `per_kind` of every resource kind already on the card.
    pub fn grow(&mut self, entity: EntityId, per_kind: u32) -> Result<(), StateError> {
        let kinds: Vec<_> = self
            .entity(entity)?
            .pool
            .iter()
            .filter(|(_, s)| *s != Supply::Finite(0))
            .map(|(r, _)| r)
            .collect();
        for res in kinds {
            self.gain_entity(entity, res, per_kind)?;
        }
        Ok(())
    }

    /// A counter by id and owner.
    pub fn counter(&self, id: CounterId, owner: &Owner) -> Option<&Counter> {
        self.counters.iter().find(|c| c.id == id && c.owner == *owner)
    }

    /// Adds to a counter (clamped); returns the applied change, or `None` when
    /// the counter does not exist.
    pub fn add_to_counter(&mut self, id: CounterId, owner: &Owner, delta: i64) -> Option<i64> {
        let applied = self
            .counters
            .iter_mut()
            .find(|c| c.id == id && c.owner == *owner)
            .map(|c| c.add(delta))?;
        if applied != 0 {
            self.emit(Event::CounterChanged {
                counter: id,
                owner:   owner.clone(),
                delta:   applied,
            });
        }
        Some(applied)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DECK: ZoneKind = ZoneKind(0);
    const HAND: ZoneKind = ZoneKind(1);
    const DISCARD: ZoneKind = ZoneKind(2);
    const P0: PlayerId = PlayerId(0);
    const COIN: ResourceId = ResourceId(0);

    /// Player 0 with a deck of `n` cards, an empty hand and discard (discard
    /// recycles into the deck).
    fn setup(n: usize, seed: u64) -> (GameState, [ZoneId; 3]) {
        let mut s = GameState::new(Rng::from_seed(seed));
        s.add_def(CardDef {
            id:        DefId(0),
            name:      "card".into(),
            tags:      vec![],
            cost:      None,
            on_gain:   None,
            abilities: vec![],
            discounts: vec![],
        });
        s.add_player(P0);
        let o = Owner::Player(P0);
        let deck = s.add_zone(DECK, o.clone(), Order::Stack, Visibility::Hidden);
        let hand = s.add_zone(HAND, o.clone(), Order::Unordered, Visibility::Owner);
        let discard = s.add_zone(DISCARD, o, Order::Stack, Visibility::Public);
        s.zone_mut(deck).unwrap().recycle_from = Some(DISCARD);
        for _ in 0..n {
            s.create_entity(DefId(0), deck, false).unwrap();
        }
        (s, [deck, hand, discard])
    }

    #[test]
    fn move_entity_keeps_zone_and_entity_in_sync() {
        let (mut s, [deck, hand, _]) = setup(3, 1);
        let top = s.zone(deck).unwrap().top().unwrap();
        s.move_entity(top, hand, Pos::Top).unwrap();
        assert_eq!(s.entity(top).unwrap().zone, hand);
        assert!(s.zone(hand).unwrap().contains(top));
        assert!(!s.zone(deck).unwrap().contains(top));
        assert_eq!(s.zone(deck).unwrap().len(), 2);
    }

    #[test]
    fn failed_move_changes_nothing() {
        let (mut s, [deck, hand, _]) = setup(2, 1);
        let top = s.zone(deck).unwrap().top().unwrap();
        assert_eq!(
            s.move_entity(top, hand, Pos::Index(5)),
            Err(StateError::BadPosition)
        );
        assert_eq!(
            s.move_entity(top, ZoneId(99), Pos::Top),
            Err(StateError::NoZone(ZoneId(99)))
        );
        assert_eq!(s.entity(top).unwrap().zone, deck);
        assert_eq!(s.zone(deck).unwrap().len(), 2);
    }

    #[test]
    fn shuffle_is_deterministic_and_keeps_all_entities() {
        let (mut a, [da, ..]) = setup(20, 5);
        let (mut b, [db, ..]) = setup(20, 5);
        let before = a.zone(da).unwrap().entities().to_vec();
        a.shuffle(da).unwrap();
        b.shuffle(db).unwrap();
        assert_eq!(
            a.zone(da).unwrap().entities(),
            b.zone(db).unwrap().entities()
        );
        assert_ne!(a.zone(da).unwrap().entities(), before.as_slice());
        let mut sorted = a.zone(da).unwrap().entities().to_vec();
        sorted.sort();
        assert_eq!(sorted, before);
    }

    #[test]
    fn draw_moves_from_the_top() {
        let (mut s, [deck, hand, _]) = setup(5, 2);
        let expected: Vec<_> =
            s.zone(deck).unwrap().entities().iter().rev().take(2).copied().collect();
        assert_eq!(s.draw(&Owner::Player(P0), DECK, HAND, 2).unwrap(), 2);
        assert_eq!(s.zone(hand).unwrap().entities(), expected.as_slice());
    }

    #[test]
    fn draw_reshuffles_discard_when_deck_runs_out() {
        let (mut s, [deck, hand, discard]) = setup(3, 4);
        let o = Owner::Player(P0);
        s.draw(&o, DECK, HAND, 3).unwrap();
        for e in s.zone(hand).unwrap().entities().to_vec() {
            s.move_entity(e, discard, Pos::Top).unwrap();
        }
        assert!(s.zone(deck).unwrap().is_empty());
        assert_eq!(s.draw(&o, DECK, HAND, 2).unwrap(), 2);
        assert_eq!(s.zone(hand).unwrap().len(), 2);
        assert_eq!(s.zone(deck).unwrap().len(), 1);
        assert!(s.zone(discard).unwrap().is_empty());
    }

    #[test]
    fn draw_stops_when_nothing_is_left_anywhere() {
        let (mut s, _) = setup(2, 4);
        assert_eq!(s.draw(&Owner::Player(P0), DECK, HAND, 5).unwrap(), 2);
    }

    #[test]
    fn draw_without_recycle_does_not_touch_discard() {
        let (mut s, [deck, hand, discard]) = setup(1, 4);
        s.zone_mut(deck).unwrap().recycle_from = None;
        let e = s.zone(deck).unwrap().top().unwrap();
        s.move_entity(e, discard, Pos::Top).unwrap();
        assert_eq!(s.draw(&Owner::Player(P0), DECK, HAND, 1).unwrap(), 0);
        assert!(s.zone(hand).unwrap().is_empty());
    }

    #[test]
    fn unknown_zone_kind_is_an_error() {
        let (s, _) = setup(0, 1);
        assert_eq!(
            s.zone_id(&Owner::Shared, DECK),
            Err(StateError::NoZoneKind(Owner::Shared, DECK))
        );
    }

    #[test]
    fn creating_an_entity_needs_a_known_def() {
        let (mut s, [deck, ..]) = setup(0, 1);
        assert_eq!(
            s.create_entity(DefId(7), deck, true),
            Err(StateError::NoDef(DefId(7)))
        );
    }

    #[test]
    fn gains_are_capped_by_the_bank_and_costs_return_to_it() {
        let (mut s, _) = setup(1, 1);
        s.bank.set(COIN, 3);
        assert_eq!(s.gain_player(P0, COIN, 5).unwrap(), 3);
        assert_eq!(s.player_pool(P0).unwrap().finite(COIN), Some(3));
        assert_eq!(s.bank.finite(COIN), Some(0));
        s.pay_player(P0, COIN, 2).unwrap();
        assert_eq!(s.bank.finite(COIN), Some(2));
        assert!(s.pay_player(P0, COIN, 2).is_err());
    }

    #[test]
    fn resources_can_sit_on_cards() {
        let (mut s, [deck, ..]) = setup(1, 1);
        s.bank.set_infinite(COIN);
        let e = s.zone(deck).unwrap().top().unwrap();
        s.gain_entity(e, COIN, 2).unwrap();
        assert_eq!(s.entity(e).unwrap().pool.finite(COIN), Some(2));
    }

    #[test]
    fn counters_are_found_by_id_and_owner() {
        let (mut s, _) = setup(0, 1);
        let vp = CounterId(0);
        s.add_counter(Counter::new(vp, Owner::Player(P0), 0, 0, 5));
        assert_eq!(s.add_to_counter(vp, &Owner::Player(P0), 9), Some(5));
        assert_eq!(s.add_to_counter(vp, &Owner::Shared, 1), None);
        assert_eq!(s.counter(vp, &Owner::Player(P0)).unwrap().value, 5);
    }

    #[test]
    fn mutations_queue_events_in_order() {
        let (mut s, [deck, hand, _]) = setup(2, 1);
        s.bank.set_infinite(COIN);
        s.add_counter(Counter::new(CounterId(0), Owner::Player(P0), 0, 0, 5));
        let top = s.zone(deck).unwrap().top().unwrap();
        s.move_entity(top, hand, Pos::Top).unwrap();
        s.gain_player(P0, COIN, 2).unwrap();
        s.pay_player(P0, COIN, 1).unwrap();
        s.add_to_counter(CounterId(0), &Owner::Player(P0), 3);
        s.add_to_counter(CounterId(0), &Owner::Player(P0), 0);
        let events: Vec<_> = std::iter::from_fn(|| s.pop_event()).collect();
        assert_eq!(
            events,
            vec![
                Event::EntityMoved {
                    entity: top,
                    from:   deck,
                    to:     hand,
                },
                Event::ResourceGained {
                    to:  Subject::Player(P0),
                    res: COIN,
                    n:   2,
                },
                Event::ResourcePaid {
                    player: P0,
                    res:    COIN,
                    n:      1,
                },
                Event::CounterChanged {
                    counter: CounterId(0),
                    owner:   Owner::Player(P0),
                    delta:   3,
                },
            ]
        );
    }

    #[test]
    fn state_survives_serde_and_keeps_the_rng_stream() {
        let (mut a, _) = setup(10, 8);
        a.bank.set_infinite(COIN);
        let mut b: GameState = serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
        assert_eq!(a, b);
        let z = a.zone_id(&Owner::Player(P0), DECK).unwrap();
        a.shuffle(z).unwrap();
        b.shuffle(z).unwrap();
        assert_eq!(a, b);
    }
}
