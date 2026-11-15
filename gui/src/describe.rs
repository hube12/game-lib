//! Turns engine data (effects, costs, payments, events) into text a player can
//! read, using the names a game provides. The matches are exhaustive on
//! purpose: a new engine effect forces an update here.

use gamelib::{
    card::CardDef,
    cost::{Cost, Count, Payment, Which},
    effect::{Condition, Defense, Effect, Filter, Limit, OwnerRef, Target, Trigger, Who},
    event::Event,
    id::{EntityId, PlayerId},
    modifier::Subject,
    state::GameState,
    zone::{Owner, Visibility},
};

use crate::{
    board::{CardDto, amounts},
    description::Names,
};

/// Marks the start of a term in marked-up text: `START kind:id SEP text END`.
pub const TERM_START: char = '\u{1}';
/// Separates a term's kind and id from its text.
pub const TERM_SEP: char = '\u{2}';
/// Marks the end of a term.
pub const TERM_END: char = '\u{3}';

/// The text without the term markers.
pub fn plain(markup: &str) -> String {
    let mut out = String::with_capacity(markup.len());
    let mut in_key = false;
    for c in markup.chars() {
        match c {
            TERM_START => in_key = true,
            TERM_SEP => in_key = false,
            TERM_END => {},
            _ if in_key => {},
            _ => out.push(c),
        }
    }
    out
}

/// The name players see for a seat.
pub fn player_name(player: PlayerId) -> String {
    format!("Player {}", u16::from(player.0) + 1)
}

/// Describes engine data against one game state.
#[derive(Clone, Copy, Debug)]
pub struct Describer<'a> {
    state:  &'a GameState,
    names:  &'a Names,
    viewer: Option<PlayerId>,
    markup: bool,
}

impl<'a> Describer<'a> {
    /// A describer that names every card (for tools and tests).
    pub const fn new(state: &'a GameState, names: &'a Names) -> Self {
        Self {
            state,
            names,
            viewer: None,
            markup: false,
        }
    }

    /// A describer that only names the cards `viewer` may see.
    pub const fn for_viewer(state: &'a GameState, names: &'a Names, viewer: PlayerId) -> Self {
        Self {
            state,
            names,
            viewer: Some(viewer),
            markup: false,
        }
    }

    /// The same describer, but marking card, zone, resource, counter and phase
    /// names so a page can underline them and explain them (see [`plain`]).
    #[must_use]
    pub const fn with_markup(mut self) -> Self {
        self.markup = true;
        self
    }

    /// A name, marked as a term of this kind when markup is on.
    fn term(&self, kind: &str, id: u32, text: String) -> String {
        if self.markup {
            format!("{TERM_START}{kind}:{id}{TERM_SEP}{text}{TERM_END}")
        } else {
            text
        }
    }

    fn zone_name(&self, kind: gamelib::id::ZoneKind) -> String {
        self.term("zone", u32::from(kind.0), self.names.zone(kind))
    }

    fn resource_name(&self, id: gamelib::id::ResourceId) -> String {
        self.term("res", u32::from(id.0), self.names.resource(id))
    }

    fn counter_name(&self, id: gamelib::id::CounterId) -> String {
        self.term("counter", u32::from(id.0), self.names.counter(id))
    }

    fn phase_name(&self, id: gamelib::id::PhaseId) -> String {
        self.term("phase", u32::from(id.0), self.names.phase(id))
    }

    /// Whether the viewer may know which card this is: the same rule as the
    /// engine's `PlayerView`.
    fn known(&self, entity: &gamelib::card::Entity) -> bool {
        let Some(viewer) = self.viewer else {
            return true;
        };
        let Ok(zone) = self.state.zone(entity.zone) else {
            return false;
        };
        match zone.visibility {
            Visibility::Public => entity.face_up || zone.owner.contains(viewer),
            Visibility::Owner => zone.owner.contains(viewer),
            Visibility::Hidden => false,
        }
    }

    /// The name of a card instance, or "a hidden card" when the viewer may not
    /// know it.
    pub fn entity(&self, id: EntityId) -> String {
        match self.state.entity(id) {
            Ok(e) if self.known(e) => self.term("card", e.def.0, self.names.card(e.def)),
            Ok(_) => "a hidden card".to_owned(),
            Err(_) => "a card".to_owned(),
        }
    }

    fn amount(&self, res: gamelib::id::ResourceId, n: u32) -> String {
        format!("{n} {}", self.resource_name(res))
    }

    fn target(&self, target: Target) -> String {
        match target {
            Target::Controller => "you".to_owned(),
            Target::Source => "this card".to_owned(),
            Target::Player(p) => player_name(p),
            Target::Entity(e) => self.entity(e),
            Target::Card => "the chosen card".to_owned(),
        }
    }

    fn on(&self, target: Target) -> String {
        match target {
            Target::Controller => String::new(),
            Target::Source => " on this card".to_owned(),
            other => format!(" on {}", self.target(other)),
        }
    }

    fn owner(owner: &OwnerRef) -> String {
        match owner {
            OwnerRef::Controller => "your".to_owned(),
            OwnerRef::Shared => "the shared".to_owned(),
            OwnerRef::Player(p) => format!("{}'s", player_name(*p)),
            OwnerRef::Players(team) => {
                let names: Vec<_> = team.iter().map(|p| player_name(*p)).collect();
                format!("{}'s", names.join(" & "))
            },
        }
    }

    fn count(count: Count) -> String {
        match count {
            Count::Fixed(n) => n.to_string(),
            Count::Var => "X".to_owned(),
            Count::CostOf { discarded, bonus } => {
                let which = if discarded { "discarded" } else { "destroyed" };
                let plus = if bonus > 0 {
                    format!(" + {bonus}")
                } else {
                    String::new()
                };
                format!("as many as the cost of the {which} card{plus}")
            },
        }
    }

    fn filter(filter: &Filter) -> String {
        let mut parts = Vec::new();
        if let Some(tag) = &filter.tag {
            parts.push(tag.clone());
        }
        match filter.exhausted {
            Some(true) => parts.push("turned".to_owned()),
            Some(false) => parts.push("ready".to_owned()),
            None => {},
        }
        match filter.has_resources {
            Some(true) => parts.push("holding resources".to_owned()),
            Some(false) => parts.push("empty".to_owned()),
            None => {},
        }
        if parts.is_empty() {
            "card".to_owned()
        } else {
            format!("{} card", parts.join(" "))
        }
    }

    fn defense(&self, defense: &Defense) -> String {
        let mut ways = Vec::new();
        if !defense.react.is_empty() {
            ways.push("a reaction".to_owned());
        }
        if let Some(escape) = &defense.escape {
            ways.push(match escape {
                gamelib::effect::Escape::Pay(res, n) => format!("paying {}", self.amount(*res, *n)),
                gamelib::effect::Escape::DiscardCard => "discarding a card".to_owned(),
                gamelib::effect::Escape::Destroy(tag) => format!("destroying a {tag}"),
            });
        }
        if ways.is_empty() {
            String::new()
        } else {
            format!(" (can be avoided by {})", ways.join(" or "))
        }
    }

    /// Describes what an effect does.
    #[allow(clippy::too_many_lines)] // one short arm per engine effect
    pub fn effect(&self, effect: &Effect) -> String {
        match effect {
            Effect::Gain { to, res, n } => {
                format!("gain {}{}", self.amount(*res, *n), self.on(*to))
            },
            Effect::GainX { to, res } => {
                format!("gain X {}{}", self.resource_name(*res), self.on(*to))
            },
            Effect::GainChoose {
                to,
                n,
                exclude,
                identical,
            } => {
                let except = if exclude.is_empty() {
                    String::new()
                } else {
                    let list: Vec<_> = exclude.iter().map(|r| self.resource_name(*r)).collect();
                    format!(" except {}", list.join("/"))
                };
                let same = if *identical { " identical" } else { "" };
                format!(
                    "gain {}{same} of any kind{except}{}",
                    Self::count(*n),
                    self.on(*to)
                )
            },
            Effect::GainPaid { to } => format!("gain the resource you paid{}", self.on(*to)),
            Effect::GainAll { who, res, n } => {
                let who = match who {
                    Who::Rivals => "your rivals",
                    Who::Everyone => "everyone",
                };
                format!("{who} gain {}", self.amount(*res, *n))
            },
            Effect::GainPerRival { res, counted } => format!(
                "gain {} for each {} a chosen rival has",
                self.resource_name(*res),
                self.resource_name(*counted)
            ),
            Effect::Pay { res, n } => format!("pay {}", self.amount(*res, *n)),
            Effect::Draw { who, n, .. } => match who {
                OwnerRef::Controller => format!("draw {n} card(s)"),
                other => format!("{} draw {n} card(s)", Self::owner(other)),
            },
            Effect::Move { entity, to, .. } => {
                format!("move {} to {}", self.target(*entity), self.zone_name(*to))
            },
            Effect::MoveAll { from, to, .. } => {
                format!(
                    "move everything from {} to {}",
                    self.zone_name(*from),
                    self.zone_name(*to)
                )
            },
            Effect::MoveNth {
                from,
                index,
                to,
                owner,
            } => {
                // Decision options resolve for the viewer, so "your" zone is
                // theirs.
                let card = match (self.viewer, owner) {
                    (Some(viewer), OwnerRef::Controller) => self
                        .state
                        .zone_id(&Owner::Player(viewer), *from)
                        .ok()
                        .and_then(|z| self.state.zone(z).ok())
                        .and_then(|z| z.entities().get(*index).copied())
                        .map(|e| self.entity(e)),
                    _ => None,
                }
                .unwrap_or_else(|| format!("card {}", index + 1));
                format!(
                    "move {card} from {} to {}",
                    self.zone_name(*from),
                    self.zone_name(*to)
                )
            },
            Effect::ModifyCounter { counter, delta, .. } => {
                format!("{delta:+} {}", self.counter_name(*counter))
            },
            Effect::ResetCounters { counter } => format!("reset {}", self.counter_name(*counter)),
            Effect::AddModifier { target, .. } => {
                format!("change a rule for {}", self.target(*target))
            },
            Effect::Exhaust(t) => format!("turn {}", self.target(*t)),
            Effect::Ready(t) => format!("straighten {}", self.target(*t)),
            Effect::ReadyAll { owner } => format!("straighten all {} cards", Self::owner(owner)),
            Effect::TakeAll { from } => format!("take everything from {}", self.target(*from)),
            Effect::Grow { per_kind } => format!("add {per_kind} of each kind already here"),
            Effect::Destroy { target } => format!("destroy {}", self.target(*target)),
            Effect::Choose(options) => {
                let list: Vec<_> = options.iter().map(|o| self.effect(o)).collect();
                format!("choose: {}", list.join(" / "))
            },
            Effect::Sequence(steps) => {
                let list: Vec<_> = steps.iter().map(|s| self.effect(s)).collect();
                list.join(", then ")
            },
            Effect::ForEachPlayer(inner) => format!("each player: {}", self.effect(inner)),
            Effect::ForEachOwned { filter, effect } => {
                format!(
                    "each of your {}s: {}",
                    Self::filter(filter),
                    self.effect(effect)
                )
            },
            Effect::Pick {
                from,
                owner,
                filter,
                then,
            } => format!(
                "pick a {} in {} {}: {}",
                Self::filter(filter),
                Self::owner(owner),
                self.zone_name(*from),
                self.effect(then)
            ),
            Effect::WithCard { card, effect } => {
                format!("{}: {}", self.entity(*card), self.effect(effect))
            },
            Effect::Affect {
                who,
                defense,
                effect,
            } => {
                let who = match who {
                    Who::Rivals => "rivals",
                    Who::Everyone => "everyone",
                };
                format!("{who}: {}{}", self.effect(effect), self.defense(defense))
            },
            Effect::AffectOne {
                player,
                defense,
                effect,
            } => format!(
                "{}: {}{}",
                player_name(*player),
                self.effect(effect),
                self.defense(defense)
            ),
            Effect::Lose { res, n, shortfall } => format!(
                "lose {}; each one missing costs {shortfall} other resources",
                self.amount(*res, *n)
            ),
            Effect::Use {
                source,
                ability,
                payment,
            } => {
                let text = self.ability(*source, *ability);
                format!(
                    "use {}: {text} [{}]",
                    self.entity(*source),
                    self.payment(payment)
                )
            },
            Effect::Ignore => "do nothing".to_owned(),
            Effect::ReactWindow { kind, .. } => format!("a reaction window ({})", kind.0),
            Effect::CheckWin => "check whether somebody won".to_owned(),
            Effect::Shuffle { zone, .. } => format!("shuffle {}", self.zone_name(*zone)),
            Effect::Rotate { zone, step } => format!(
                "everybody passes {} to the {} player",
                self.zone_name(*zone),
                if *step < 0 { "previous" } else { "next" }
            ),
            Effect::Simultaneous(_) => "everybody chooses at the same time".to_owned(),
            Effect::Reorder { zone, n, .. } => {
                format!(
                    "look at the top {n} of {} and put them back in any order",
                    self.zone_name(*zone)
                )
            },
            Effect::Permute { perm, .. } => {
                let order: Vec<_> = perm.iter().map(|i| (i + 1).to_string()).collect();
                format!("order {}", order.join(" "))
            },
            Effect::PlaceCard {
                from,
                tag,
                discount,
            } => {
                let from = from.map_or("your hand".to_owned(), |z| self.zone_name(z));
                let tag = tag.as_deref().unwrap_or("card");
                let off = discount.as_ref().map_or(String::new(), |d| {
                    if d.n >= 1_000 {
                        ", free".to_owned()
                    } else {
                        format!(", {} cheaper", d.n)
                    }
                });
                format!("place {} from {from}{off}", Self::article(tag))
            },
            Effect::DiscardChoose { n } => format!("discard {n} card(s) of your choice"),
            Effect::DrawDiscard { n, .. } => {
                format!("draw {n} card(s), then discard as many cards of your choice")
            },
        }
    }

    /// The cards an effect is about, as the viewer may see them (a card they
    /// may not see has no `def`), in the order they appear, without repeats.
    pub fn cards_in(&self, effect: &Effect) -> Vec<CardDto> {
        let mut ids = Vec::new();
        self.collect_cards(effect, &mut ids);
        ids.into_iter()
            .filter_map(|id| {
                let e = self.state.entity(id).ok()?;
                Some(CardDto {
                    id:        id.0,
                    def:       self.known(e).then_some(e.def.0),
                    face_up:   e.face_up,
                    exhausted: e.exhausted,
                    pool:      amounts(&e.pool),
                })
            })
            .collect()
    }

    fn mention(ids: &mut Vec<EntityId>, id: EntityId) {
        if !ids.contains(&id) {
            ids.push(id);
        }
    }

    fn mention_target(ids: &mut Vec<EntityId>, target: Target) {
        if let Target::Entity(e) = target {
            Self::mention(ids, e);
        }
    }

    // Only effects that can point at a card are listed; the rest name none.
    fn collect_cards(&self, effect: &Effect, ids: &mut Vec<EntityId>) {
        match effect {
            Effect::WithCard { card, effect } => {
                Self::mention(ids, *card);
                self.collect_cards(effect, ids);
            },
            Effect::Move { entity, .. } => Self::mention_target(ids, *entity),
            Effect::Gain { to, .. }
            | Effect::GainX { to, .. }
            | Effect::GainPaid { to }
            | Effect::GainChoose { to, .. } => {
                Self::mention_target(ids, *to);
            },
            Effect::Exhaust(t) | Effect::Ready(t) => Self::mention_target(ids, *t),
            Effect::TakeAll { from } => Self::mention_target(ids, *from),
            Effect::Destroy { target } => Self::mention_target(ids, *target),
            Effect::MoveNth {
                from,
                index,
                owner: OwnerRef::Controller,
                ..
            } => {
                // Decision options resolve for the viewer: their zone's card at
                // that position.
                let card = self.viewer.and_then(|viewer| {
                    let zone = self.state.zone_id(&Owner::Player(viewer), *from).ok()?;
                    self.state.zone(zone).ok()?.entities().get(*index).copied()
                });
                if let Some(card) = card {
                    Self::mention(ids, card);
                }
            },
            Effect::Use {
                source, payment, ..
            } => {
                Self::mention(ids, *source);
                for e in payment.turned.iter().chain(&payment.discarded).chain(&payment.destroyed) {
                    Self::mention(ids, *e);
                }
            },
            Effect::Choose(list) | Effect::Sequence(list) | Effect::Simultaneous(list) => {
                for e in list {
                    self.collect_cards(e, ids);
                }
            },
            Effect::ForEachPlayer(inner)
            | Effect::ForEachOwned { effect: inner, .. }
            | Effect::Pick { then: inner, .. }
            | Effect::Affect { effect: inner, .. }
            | Effect::AffectOne { effect: inner, .. } => self.collect_cards(inner, ids),
            Effect::ReactWindow { default, .. } => self.collect_cards(default, ids),
            _ => {},
        }
    }

    /// "a card" / "an artifact".
    fn article(noun: &str) -> String {
        let vowel = noun.chars().next().is_some_and(|c| "aeiouAEIOU".contains(c));
        format!("{} {noun}", if vowel { "an" } else { "a" })
    }

    fn condition(&self, condition: &Condition) -> Option<String> {
        match condition {
            Condition::Always => None,
            Condition::OwnTurn => Some("on your turn".to_owned()),
            Condition::OtherTurn => Some("on another player's turn".to_owned()),
            Condition::InPhase(phase) => Some(format!("during {}", self.phase_name(*phase))),
            Condition::SourceHas(res) => {
                Some(format!("while this card has {}", self.resource_name(*res)))
            },
            Condition::SourceHasResources => Some("while this card holds resources".to_owned()),
            Condition::All(list) => {
                let parts: Vec<_> = list.iter().filter_map(|c| self.condition(c)).collect();
                (!parts.is_empty()).then(|| parts.join(" and "))
            },
        }
    }

    /// What every ability of a card does, in English: its powers, collect and
    /// automatic abilities, reactions, placement discounts and what happens
    /// when it is claimed.
    pub fn card_text(&self, def: &CardDef) -> Vec<String> {
        let mut lines = Vec::new();
        for ability in &def.abilities {
            let kind = match ability.trigger {
                Trigger::Activated => "Power",
                Trigger::OnEvent(_) => "Automatic",
                Trigger::React(_) => "Reaction",
            };
            let when =
                self.condition(&ability.condition).map_or(String::new(), |c| format!(" ({c})"));
            let cost = ability
                .cost
                .as_ref()
                .map_or(String::new(), |c| format!("{} \u{2192} ", self.cost(c)));
            let limit = match ability.limit {
                Limit::None => "",
                Limit::OncePerTurn => " (once per turn)",
                Limit::OncePerRound => " (once per round)",
                Limit::OncePerBetweenTurns => " (once between your turns)",
            };
            lines.push(format!(
                "{kind}{when}: {cost}{}{limit}",
                self.effect(&ability.effect)
            ));
        }
        for discount in &def.discounts {
            let what = discount.tag.as_deref().unwrap_or("card");
            let except = if discount.exclude.is_empty() {
                String::new()
            } else {
                let list: Vec<_> =
                    discount.exclude.iter().map(|r| self.resource_name(*r)).collect();
                format!(" (not {})", list.join("/"))
            };
            let amount = if discount.n >= 1_000 {
                "free".to_owned()
            } else {
                format!("{} cheaper", discount.n)
            };
            lines.push(format!(
                "Discount: placing {} is {amount}{except}",
                Self::article(what)
            ));
        }
        if let Some(effect) = &def.on_gain {
            lines.push(format!("When claimed: {}", self.effect(effect)));
        }
        lines
    }

    /// What an ability of a card does, with its cost.
    pub fn ability(&self, entity: EntityId, index: usize) -> String {
        let Ok(card) = self.state.entity(entity) else {
            return "?".to_owned();
        };
        let Some(ability) = self.state.defs.get(&card.def).and_then(|d| d.abilities.get(index))
        else {
            return "?".to_owned();
        };
        self.effect(&ability.effect)
    }

    /// Describes a cost.
    pub fn cost(&self, cost: &Cost) -> String {
        match cost {
            Cost::Resources(parts) => {
                let list: Vec<_> = parts.iter().map(|(r, n)| self.amount(*r, *n)).collect();
                if list.is_empty() {
                    "free".to_owned()
                } else {
                    list.join(", ")
                }
            },
            Cost::Or(options) => {
                let list: Vec<_> = options.iter().map(|c| self.cost(c)).collect();
                list.join(" or ")
            },
            Cost::All(parts) => {
                let list: Vec<_> = parts.iter().map(|c| self.cost(c)).collect();
                list.join(" + ")
            },
            Cost::Exhaust => "turn".to_owned(),
            Cost::Any {
                n,
                exclude,
                identical,
            } => {
                let except = if exclude.is_empty() {
                    String::new()
                } else {
                    let list: Vec<_> = exclude.iter().map(|r| self.resource_name(*r)).collect();
                    format!(" except {}", list.join("/"))
                };
                let same = if *identical { " identical" } else { "" };
                format!("any {} resource(s){same}{except}", Self::count(*n))
            },
            Cost::TakeFromCard { res, n } => {
                format!("take {} off this card", self.amount(*res, *n))
            },
            Cost::TurnOther { tag } => format!("turn another {tag}"),
            Cost::Destroy { which, tag } => {
                let which = match which {
                    Which::This => "this card",
                    Which::Other => "another",
                    Which::Own => "one of yours:",
                };
                format!("destroy {which} {tag}")
            },
            Cost::Discard { n, .. } => format!("discard {n} card(s)"),
        }
    }

    /// Describes one concrete way of paying.
    pub fn payment(&self, payment: &Payment) -> String {
        let mut parts: Vec<String> =
            payment.resources.iter().map(|(r, n)| self.amount(*r, *n)).collect();
        if payment.exhaust {
            parts.push("turn it".to_owned());
        }
        parts.extend(
            payment
                .from_card
                .iter()
                .map(|(r, n)| format!("take {} off the card", self.amount(*r, *n))),
        );
        parts.extend(payment.turned.iter().map(|e| format!("turn {}", self.entity(*e))));
        if let Some(card) = payment.destroyed {
            parts.push(format!("destroy {}", self.entity(card)));
        }
        parts.extend(payment.discarded.iter().map(|e| format!("discard {}", self.entity(*e))));
        if let Some(x) = payment.x {
            parts.push(format!("X = {x}"));
        }
        if parts.is_empty() {
            "free".to_owned()
        } else {
            parts.join(", ")
        }
    }

    /// What `viewer` may read about a card that moved to a zone: its name when
    /// they can see where it went, otherwise "a card".
    fn moved_card(
        &self,
        viewer: PlayerId,
        entity: EntityId,
        to: gamelib::id::ZoneId,
    ) -> Option<String> {
        let zone = self.state.zone(to).ok()?;
        let visible = match zone.visibility {
            Visibility::Public => true,
            Visibility::Owner => zone.owner.contains(viewer),
            Visibility::Hidden => false,
        };
        visible.then(|| self.entity(entity))
    }

    fn owner_label(owner: &Owner) -> String {
        match owner {
            Owner::Shared => "the table".to_owned(),
            Owner::Player(p) => player_name(*p),
            Owner::Players(team) => {
                let names: Vec<_> = team.iter().map(|p| player_name(*p)).collect();
                names.join(" & ")
            },
        }
    }

    /// A log line for an event, as `viewer` may see it; `None` for events that
    /// are not worth showing (hidden moves, bookkeeping).
    pub fn event(&self, viewer: PlayerId, event: &Event) -> Option<(Option<u8>, String)> {
        let line = match event {
            Event::RoundStarted(n) => (None, format!("— Round {n} —")),
            Event::PhaseStarted(phase) => (None, self.phase_name(*phase)),
            Event::TurnStarted(p) => (Some(p.0), format!("{}'s turn", player_name(*p))),
            Event::Passed(p) => (Some(p.0), format!("{} passed", player_name(*p))),
            Event::CardBought { player, entity } => (
                Some(player.0),
                format!("{} claimed {}", player_name(*player), self.entity(*entity)),
            ),
            Event::AbilityActivated {
                player,
                entity,
                ability,
            } => (
                Some(player.0),
                format!(
                    "{} used {}: {}",
                    player_name(*player),
                    self.entity(*entity),
                    self.ability(*entity, *ability)
                ),
            ),
            Event::EntityMoved { entity, to, .. } => {
                let zone = self.state.zone(*to).ok()?;
                let name = self.moved_card(viewer, *entity, *to)?;
                let player = match &zone.owner {
                    Owner::Player(p) => Some(p.0),
                    _ => None,
                };
                (
                    player,
                    format!(
                        "{name} → {} ({})",
                        self.zone_name(zone.kind),
                        Self::owner_label(&zone.owner)
                    ),
                )
            },
            Event::ResourceGained { to, res, n } => {
                let who = match to {
                    Subject::Player(p) => player_name(*p),
                    Subject::Entity(e) => self.entity(*e),
                };
                let player = match to {
                    Subject::Player(p) => Some(p.0),
                    Subject::Entity(_) => None,
                };
                (player, format!("{who} gained {}", self.amount(*res, *n)))
            },
            Event::ResourcePaid { player, res, n } => (
                Some(player.0),
                format!("{} paid {}", player_name(*player), self.amount(*res, *n)),
            ),
            Event::CounterChanged {
                counter,
                owner,
                delta,
            } => (
                match owner {
                    Owner::Player(p) => Some(p.0),
                    _ => None,
                },
                format!(
                    "{} {delta:+} {}",
                    Self::owner_label(owner),
                    self.counter_name(*counter)
                ),
            ),
            Event::GameEnded(winners) => {
                let names: Vec<_> = winners.iter().map(|p| player_name(*p)).collect();
                (None, format!("Game over — {} won", names.join(" & ")))
            },
            Event::RoundEnded(_)
            | Event::TurnEnded(_)
            | Event::Shuffled(_)
            | Event::DecisionRequested(_)
            | Event::DecisionMade(_) => return None,
        };
        Some(line)
    }
}

#[cfg(test)]
mod tests {
    use gamelib::{
        id::{ResourceId, ZoneKind},
        rng::Rng,
    };

    use super::*;
    use crate::description::{GameDescription, NamedId, ResourceInfo};

    fn names() -> Names {
        Names::new(&GameDescription {
            title:       "t".into(),
            min_players: 1,
            max_players: 2,
            resources:   vec![ResourceInfo {
                id:     0,
                name:   "Gold".into(),
                symbol: "G".into(),
                color:  "#fc0".into(),
                help:   String::new(),
            }],
            zones:       vec![NamedId {
                id:   1,
                name: "Hand".into(),
                help: String::new(),
            }],
            counters:    vec![],
            phases:      vec![],
            cards:       vec![],
            card_images: String::new(),
        })
    }

    #[test]
    fn effects_and_costs_read_like_the_rules() {
        let state = GameState::new(Rng::from_seed(1));
        let names = names();
        let d = Describer::new(&state, &names);
        let gold = ResourceId(0);
        assert_eq!(
            d.effect(&Effect::Gain {
                to:  Target::Source,
                res: gold,
                n:   2,
            }),
            "gain 2 Gold on this card"
        );
        assert_eq!(
            d.effect(&Effect::Sequence(vec![
                Effect::Pay { res: gold, n: 1 },
                Effect::Draw {
                    who:  OwnerRef::Controller,
                    from: ZoneKind(0),
                    to:   ZoneKind(1),
                    n:    1,
                },
            ])),
            "pay 1 Gold, then draw 1 card(s)"
        );
        assert_eq!(
            d.cost(&Cost::All(vec![
                Cost::Exhaust,
                Cost::Resources(vec![(gold, 3)])
            ])),
            "turn + 3 Gold"
        );
        assert_eq!(d.cost(&Cost::Resources(vec![])), "free");
        assert_eq!(player_name(PlayerId(0)), "Player 1");
    }

    #[test]
    fn marked_up_text_names_its_terms_and_strips_back_to_the_plain_text() {
        let state = GameState::new(Rng::from_seed(1));
        let names = names();
        let effect = Effect::Move {
            entity: Target::Card,
            to:     ZoneKind(1),
            owner:  OwnerRef::Controller,
        };
        let plain_text = Describer::new(&state, &names).effect(&effect);
        let marked = Describer::new(&state, &names).with_markup().effect(&effect);
        assert_eq!(plain_text, "move the chosen card to Hand");
        assert!(marked.contains("\u{1}zone:1\u{2}Hand\u{3}"), "{marked:?}");
        assert_eq!(plain(&marked), plain_text);
        let gain = Effect::Gain {
            to:  Target::Controller,
            res: ResourceId(0),
            n:   2,
        };
        let marked = Describer::new(&state, &names).with_markup().effect(&gain);
        assert!(
            marked.starts_with("gain 2 \u{1}res:0\u{2}Gold\u{3}"),
            "{marked:?}"
        );
    }

    #[test]
    fn payments_list_every_part() {
        let state = GameState::new(Rng::from_seed(1));
        let names = names();
        let d = Describer::new(&state, &names);
        let mut payment = Payment::default();
        assert_eq!(d.payment(&payment), "free");
        payment.resources.insert(ResourceId(0), 2);
        payment.exhaust = true;
        payment.x = Some(2);
        assert_eq!(d.payment(&payment), "2 Gold, turn it, X = 2");
    }
}
