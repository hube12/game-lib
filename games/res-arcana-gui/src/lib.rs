//! Res Arcana for `gamelib-gui`: the names, colors and printed card text the
//! page shows, the scoreboard, the bot opponents, and the page itself.
#![deny(missing_docs)]
#![warn(clippy::all, clippy::pedantic, clippy::nursery, clippy::cargo)]
#![allow(
    clippy::use_self,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc
)]

use gamelib::{bot::Bot, id::PlayerId, rng::Rng, state::GameState};
use gamelib_gui::{
    Backend,
    CardInfo,
    GameDescription,
    GameUi,
    NamedId,
    Names,
    ResourceInfo,
    ScoreLine,
    describe::Describer,
    web::Asset,
};
use gamelib_res_arcana::{
    CARDS_TEXT,
    CardSpec,
    compile::{
        ACTIONS,
        CALM,
        CLEANUP,
        COLLECT,
        COLLECT_STEP,
        DEATH,
        DECK,
        DISCARD,
        ELAN,
        GOLD,
        HAND,
        LIFE,
        MONUMENT_DECK,
        TABLEAU,
        TAKE,
        TEMP_VP,
        VICTORY,
        VP,
        kind_tag,
        take_from_card,
        type_tag,
    },
    planner::PlannerBot,
    rules::{
        ASIDE,
        CANDIDATES,
        DRAFT,
        FIRST_TOKEN,
        ITEMS,
        MONUMENT_ROW,
        PLACES,
        ResArcanaRules,
        STOCK,
        UNUSED,
    },
};

/// The page and its styles, served next to the shared components.
pub const PAGES: &[Asset] = &[
    Asset {
        path: "index.html",
        mime: "text/html; charset=utf-8",
        body: include_str!("../web/index.html"),
    },
    Asset {
        path: "app.js",
        mime: "text/javascript",
        body: include_str!("../web/app.js"),
    },
    Asset {
        path: "res-arcana.css",
        mime: "text/css",
        body: include_str!("../web/res-arcana.css"),
    },
];

/// Res Arcana as the GUI plays it.
#[derive(Clone, Debug)]
pub struct ResArcanaUi {
    rules: ResArcanaRules,
}

/// The option that turns on the draft variant (see
/// [`ResArcanaRules::with_draft`]).
pub const DRAFT_OPTION: &str = "draft";

impl ResArcanaUi {
    /// The base game.
    pub fn new() -> Self {
        Self {
            rules: ResArcanaRules::new(),
        }
    }
}

impl Default for ResArcanaUi {
    fn default() -> Self {
        Self::new()
    }
}

/// A backend with Res Arcana loaded.
pub fn backend() -> Backend<ResArcanaUi> {
    Backend::new(ResArcanaUi::new())
}

/// The environment variable that names the directory for game logs.
pub const LOG_DIR_VAR: &str = "GAMELIB_LOG_DIR";

/// Where games are logged: the directory named by [`LOG_DIR_VAR`], or else
/// `game-logs/res-arcana` in the repository this program was built from.
pub fn log_dir() -> std::path::PathBuf {
    std::env::var_os(LOG_DIR_VAR).map_or_else(
        || std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-logs/res-arcana"),
        std::path::PathBuf::from,
    )
}

/// A backend that writes every game to its own file in [`log_dir`], for
/// studying how people and the bot play.
pub fn backend_with_logs() -> Backend<ResArcanaUi> {
    backend().with_log_dir(log_dir())
}

/// The printed text of every card, in file order: `(cost, points, ability
/// lines)`.
fn printed(text: &str) -> Vec<(String, String, Vec<String>)> {
    const KINDS: [&str; 5] = ["mage ", "artifact ", "monument ", "place ", "item "];
    let mut cards: Vec<(String, String, Vec<String>)> = Vec::new();
    for line in text.lines() {
        if KINDS.iter().any(|k| line.starts_with(k)) {
            let token = |key: &str| {
                line.split_whitespace()
                    .find_map(|t| t.strip_prefix(key))
                    .unwrap_or_default()
                    .to_owned()
            };
            cards.push((token("cost="), token("vp="), Vec::new()));
        } else if line.starts_with("  ")
            && !line.trim_start().starts_with('#')
            && !line.trim().is_empty()
            && let Some(card) = cards.last_mut()
        {
            card.2.push(line.trim().to_owned());
        }
    }
    cards
}

fn card_info(
    index: usize,
    spec: &CardSpec,
    printed: Option<&(String, String, Vec<String>)>,
) -> CardInfo {
    let mut tags = vec![kind_tag(spec.kind).to_owned()];
    tags.extend(spec.types.iter().map(|t| type_tag(*t).to_owned()));
    let (name, alt) = spec.english.as_ref().map_or_else(
        || (spec.name.clone(), None),
        |english| (english.clone(), Some(spec.name.clone())),
    );
    let (cost, points, lines) = printed.cloned().unwrap_or_default();
    CardInfo {
        id: u32::try_from(index).unwrap_or(u32::MAX),
        name,
        alt,
        tags,
        cost,
        points,
        lines,
        powers: Vec::new(),
    }
}

/// What a card's abilities do, in English. The engine plays a collect ability
/// as a reaction of the collect step; the card calls it "Collect". The rule
/// that lets a player take the essences off any component is not printed on
/// cards and is left out.
fn card_powers(describer: &Describer<'_>, def: &gamelib::card::CardDef) -> Vec<String> {
    let mut kept = Vec::new();
    for (i, line) in describer.card_text(def).into_iter().enumerate() {
        let collect = def
            .abilities
            .get(i)
            .is_some_and(|a| a.trigger == gamelib::effect::Trigger::React(COLLECT_STEP));
        if !collect {
            kept.push(line);
        } else if def.abilities[i] != take_from_card() {
            let renamed = line
                .strip_prefix("Reaction")
                .map(|rest| format!("Collect{}", rest.trim_end_matches(" (once per round)")));
            kept.push(renamed.unwrap_or(line));
        }
    }
    kept
}

fn named(list: &[(u16, &str, &str)]) -> Vec<NamedId> {
    list.iter()
        .map(|(id, name, help)| NamedId {
            id:   *id,
            name: (*name).to_owned(),
            help: (*help).to_owned(),
        })
        .collect()
}

fn resources() -> Vec<ResourceInfo> {
    let resource =
        |id: gamelib::id::ResourceId, name: &str, symbol: &str, color: &str, help: &str| {
            ResourceInfo {
                id:     id.0,
                name:   name.to_owned(),
                symbol: symbol.to_owned(),
                color:  color.to_owned(),
                help:   help.to_owned(),
            }
        };
    vec![
        resource(
            ELAN,
            "Elan",
            "E",
            "#c8453a",
            "An essence (red). Powers pay it and make it.",
        ),
        resource(
            LIFE,
            "Life",
            "L",
            "#3f8f4e",
            "An essence (green). When a rival makes you lose Life, each Life you cannot lose \
             costs you any 2 other essences.",
        ),
        resource(CALM, "Calm", "C", "#3a78c2", "An essence (blue)."),
        resource(DEATH, "Death", "D", "#4a4750", "An essence (black)."),
        resource(
            GOLD,
            "Gold",
            "G",
            "#c79a1c",
            "An essence (yellow). A monument costs 4 Gold, and Gold counts double when breaking \
             ties.",
        ),
    ]
}

fn zones() -> Vec<NamedId> {
    named(&[
        (
            DECK.0,
            "Deck",
            "Your draw pile, face down. You draw from it when you pass; when it runs out, your \
             discard pile is shuffled into it.",
        ),
        (
            HAND.0,
            "Hand",
            "Artifacts only you can see. Place one by paying its cost, or discard one for 1 Gold \
             or any 2 other essences.",
        ),
        (
            DISCARD.0,
            "Discard",
            "Discarded and destroyed artifacts, face up.",
        ),
        (
            TABLEAU.0,
            "In play",
            "Everything you have in play: your mage, your magic item and the artifacts, monuments \
             and places you placed or claimed. Their powers can be used from here.",
        ),
        (
            MONUMENT_DECK.0,
            "Monument deck",
            "The monuments nobody can see. You may claim the top one without looking at it.",
        ),
        (
            PLACES.0,
            "Places of power",
            "Five places anybody can claim by paying their cost.",
        ),
        (
            MONUMENT_ROW.0,
            "Monuments",
            "The two face-up monuments. Claim one for 4 Gold; it is replaced from the deck.",
        ),
        (
            ITEMS.0,
            "Magic items",
            "Magic items nobody holds. When you pass you swap yours for one of these.",
        ),
        (
            CANDIDATES.0,
            "Mages to choose from",
            "The two mages you were dealt. Keep one; the other is set aside.",
        ),
        (UNUSED.0, "Unused mages", "Mages that were not kept."),
        (
            DRAFT.0,
            "Artifacts to draft",
            "Draft variant: keep one of these artifacts for your deck and pass the rest on (to \
             the left in the first half, to the right in the second).",
        ),
        (
            STOCK.0,
            "Artifacts still to be dealt",
            "Draft variant: your next four artifacts, dealt when the current ones are used up.",
        ),
        (
            ASIDE.0,
            "Item being swapped",
            "The magic item you gave up while you pick its replacement; it goes back to the \
             middle afterwards.",
        ),
    ])
}

fn counters() -> Vec<NamedId> {
    named(&[
        (
            VP.0,
            "Victory points",
            "Points your powers gave you. At a victory check they are added to the points printed \
             on your cards; 10 wins.",
        ),
        (
            TEMP_VP.0,
            "Temporary victory points",
            "Points that only count at the current victory check.",
        ),
        (
            FIRST_TOKEN.0,
            "First player token",
            "Held by the first player to pass last round. It is worth 1 victory point at a \
             victory check.",
        ),
    ])
}

fn phases() -> Vec<NamedId> {
    named(&[
        (
            TAKE.0,
            "Collect",
            "Each player does their collect abilities and may take the essences sitting on their \
             components (all of a component or none), in the order they like.",
        ),
        (
            COLLECT.0,
            "Collect (left on cards)",
            "The abilities that depend on what was left on a component happen (Vault, Windup Man).",
        ),
        (
            ACTIONS.0,
            "Actions",
            "Players take turns, one action each: place an artifact, claim a monument or place, \
             discard a card, use a power, or pass.",
        ),
        (
            VICTORY.0,
            "Victory check",
            "Players may react; then anybody with 10 or more victory points wins (ties: most \
             essences, Gold counts double).",
        ),
        (
            CLEANUP.0,
            "Cleanup",
            "Every component is straightened and a new round begins.",
        ),
    ])
}

impl GameUi for ResArcanaUi {
    type Rules = ResArcanaRules;

    fn rules(&self) -> ResArcanaRules {
        self.rules.clone()
    }

    fn with_options(mut self, options: &[String]) -> Self {
        self.rules = self.rules.with_draft(options.iter().any(|o| o == DRAFT_OPTION));
        self
    }

    fn description(&self) -> GameDescription {
        let texts = printed(CARDS_TEXT);
        let specs = (0..)
            .map_while(|i| self.rules.spec(gamelib::id::DefId(i)).map(|s| (i, s)))
            .collect::<Vec<_>>();
        let mut description = GameDescription {
            title:       "Res Arcana".to_owned(),
            min_players: 2,
            max_players: 4,
            resources:   resources(),
            zones:       zones(),
            counters:    counters(),
            phases:      phases(),
            cards:       specs
                .iter()
                .map(|(i, spec)| {
                    card_info(
                        usize::try_from(*i).unwrap_or(0),
                        spec,
                        texts.get(usize::try_from(*i).unwrap_or(0)),
                    )
                })
                .collect(),
            // Rough pictures cut from the owner's photos by
            // scripts/crop_cards.sh.
            card_images: "cards/{id}.jpg".to_owned(),
        };
        // What each card's abilities do, in English, from the compiled
        // definitions the engine really plays.
        let names = Names::new(&description);
        let state = GameState::new(Rng::from_seed(0));
        let describer = Describer::new(&state, &names).with_markup();
        for card in &mut description.cards {
            if let Some(def) = self.rules.cards().defs.iter().find(|d| d.id.0 == card.id) {
                card.powers = card_powers(&describer, def);
            }
        }
        description
    }

    fn bot(&self, _player: PlayerId, _seed: u64) -> Box<dyn Bot + Send> {
        Box::new(PlannerBot::new())
    }

    fn option_label(
        &self,
        state: &GameState,
        pending: &gamelib::effect::PendingDecision,
        option: &gamelib::effect::Effect,
    ) -> Option<String> {
        use gamelib::effect::Effect;
        if *option != Effect::Ignore {
            return None;
        }
        // The Vault pays interest only if its Gold is left on it: say so on
        // the option that leaves it.
        let vault_with_gold = pending.options.iter().any(|o| {
            let first = match o {
                Effect::Sequence(steps) => steps.first(),
                other => Some(other),
            };
            let Some(Effect::Use { source, .. }) = first else {
                return false;
            };
            state.entity(*source).is_ok_and(|e| {
                e.pool.finite(GOLD).unwrap_or(0) > 0
                    && state.defs.get(&e.def).is_some_and(|d| d.name == "Tresor")
            })
        });
        vault_with_gold.then(|| "leave the Gold on the Vault (it pays 2 essences)".to_owned())
    }

    fn scores(&self, state: &GameState) -> Vec<ScoreLine> {
        state
            .flow
            .turn_order
            .order()
            .iter()
            .map(|p| ScoreLine {
                player: p.0,
                label:  "VP".to_owned(),
                value:  self.rules.score(state, *p),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_card_is_shown_under_its_english_name() {
        let description = ResArcanaUi::new().description();
        // Cards whose English and German names are the same carry no second
        // name.
        let mut same: Vec<_> =
            description.cards.iter().filter(|c| c.alt.is_none()).map(|c| c.name.as_str()).collect();
        same.sort_unstable();
        assert_eq!(same, ["Alchemist", "Athanor", "Mausoleum", "Obelisk"]);
        // Everything else shows the English name and keeps the printed German
        // one next to it.
        assert!(
            description
                .cards
                .iter()
                .filter(|c| c.alt.is_some())
                .all(|c| c.alt.as_deref() != Some(c.name.as_str()))
        );
        assert!(
            description
                .cards
                .iter()
                .any(|c| c.name == "Healer" && c.alt.as_deref() == Some("Heiler"))
        );
    }

    #[test]
    fn the_description_covers_every_card_with_its_printed_text() {
        let description = ResArcanaUi::new().description();
        let rules = ResArcanaRules::new();
        assert_eq!(description.cards.len(), rules.cards().names.len());
        assert_eq!(
            printed(CARDS_TEXT).len(),
            description.cards.len(),
            "one printed block per card"
        );
        let heiler = description
            .cards
            .iter()
            .find(|c| c.alt.as_deref() == Some("Heiler"))
            .or_else(|| description.cards.iter().find(|c| c.name == "Heiler"))
            .unwrap();
        assert!(heiler.lines.iter().any(|l| l.contains("lifeloss")));
        let forge = description.cards.iter().find(|c| c.name == "Cursed Forge").unwrap();
        assert_eq!(forge.cost, "6E+3D");
        assert_eq!(forge.points, "1+1/G");
        assert!(forge.tags.contains(&"place".to_owned()));
        assert_eq!(description.resources.len(), 5);
    }
}

#[cfg(test)]
mod powers_tests {
    use gamelib_gui::describe::plain;

    use super::*;

    #[test]
    fn items_places_and_monuments_describe_their_powers_in_english() {
        let description = ResArcanaUi::new().description();
        for card in &description.cards {
            let family =
                ["item", "place", "monument"].iter().any(|t| card.tags.contains(&(*t).to_owned()));
            // A card that prints no ability (the Great Pyramid is only points)
            // has nothing to describe.
            if family {
                assert_eq!(
                    card.powers.is_empty(),
                    card.lines.is_empty(),
                    "{}: {:?}",
                    card.name,
                    card.powers
                );
            }
        }
        let text = |name: &str| {
            let card = description
                .cards
                .iter()
                .find(|c| c.name == name)
                .unwrap_or_else(|| panic!("{name}"));
            card.powers.iter().map(|l| plain(l)).collect::<Vec<_>>().join(" | ")
        };
        assert!(
            text("Coral Castle").contains("check whether somebody won"),
            "{}",
            text("Coral Castle")
        );
        assert!(text("Healer").contains("Reaction"), "{}", text("Healer"));
        assert!(
            text("Crypt").contains("place an artifact from Discard"),
            "{}",
            text("Crypt")
        );
        assert!(
            text("Artificer").contains("Discount"),
            "{}",
            text("Artificer")
        );
        assert!(
            text("Obelisk").contains("When claimed"),
            "{}",
            text("Obelisk")
        );
        println!("{}", text("Research"));
    }
}
