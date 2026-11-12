//! The typed model of a card, written with the rulebook's symbols.

use std::fmt;

/// The five essences. Each has a rulebook symbol letter and a token colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Essence {
    /// `E`, red.
    Elan,
    /// `L`, green.
    Life,
    /// `C`, blue.
    Calm,
    /// `D`, black.
    Death,
    /// `G`, yellow.
    Gold,
}

impl Essence {
    /// All essences in rulebook order.
    pub const ALL: [Self; 5] = [Self::Elan, Self::Life, Self::Calm, Self::Death, Self::Gold];

    /// The symbol letter used in the card file.
    pub const fn symbol(self) -> char {
        match self {
            Self::Elan => 'E',
            Self::Life => 'L',
            Self::Calm => 'C',
            Self::Death => 'D',
            Self::Gold => 'G',
        }
    }

    /// The essence for a symbol letter.
    pub const fn from_symbol(symbol: char) -> Option<Self> {
        match symbol {
            'E' => Some(Self::Elan),
            'L' => Some(Self::Life),
            'C' => Some(Self::Calm),
            'D' => Some(Self::Death),
            'G' => Some(Self::Gold),
            _ => None,
        }
    }

    /// The rulebook name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Elan => "Elan",
            Self::Life => "Life",
            Self::Calm => "Calm",
            Self::Death => "Death",
            Self::Gold => "Gold",
        }
    }

    /// The colour of the token.
    pub const fn color(self) -> &'static str {
        match self {
            Self::Elan => "red",
            Self::Life => "green",
            Self::Calm => "blue",
            Self::Death => "black",
            Self::Gold => "yellow",
        }
    }
}

/// How many essences: a fixed number, or a number the player chooses (`N`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Count {
    /// A printed number (no number printed means 1).
    Fixed(u32),
    /// `N`: chosen by the player when the power is used.
    Var,
}

/// Which essences an amount is made of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Selector {
    /// One specific essence.
    Essence(Essence),
    /// The grey pentagon: any essence, except the listed ones (the small
    /// markers next to it).
    Any {
        /// Essences that may not be chosen.
        exclude: Vec<Essence>,
    },
}

/// A number of essences, as printed on a card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Amount {
    /// How many.
    pub count:     Count,
    /// Of what.
    pub what:      Selector,
    /// For any-essence amounts: all of them must be the same kind (`*=`).
    pub identical: bool,
}

impl Amount {
    /// A fixed amount of one essence.
    pub const fn of(count: u32, essence: Essence) -> Self {
        Self {
            count:     Count::Fixed(count),
            what:      Selector::Essence(essence),
            identical: false,
        }
    }
}

impl fmt::Display for Amount {
    /// Writes the amount in the card file's symbol notation.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.count {
            Count::Fixed(1) => {},
            Count::Fixed(n) => write!(f, "{n}")?,
            Count::Var => f.write_str("N")?,
        }
        match &self.what {
            Selector::Essence(e) => write!(f, "{}", e.symbol()),
            Selector::Any { exclude } => {
                f.write_str("*")?;
                if self.identical {
                    f.write_str("=")?;
                }
                if !exclude.is_empty() {
                    f.write_str("-")?;
                    for e in exclude {
                        write!(f, "{}", e.symbol())?;
                    }
                }
                Ok(())
            },
        }
    }
}

/// Alternatives; the player picks one group and gets everything in it (`C/L` is
/// two groups, `D+E` one).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options(pub Vec<Vec<Amount>>);

/// Card families.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CardKind {
    /// Mage: begins in play.
    Mage,
    /// Artifact: placed from hand.
    Artifact,
    /// Monument: claimed for 4 Gold.
    Monument,
    /// Place of Power: claimed from the centre.
    Place,
    /// Magic item: chosen in setup, swapped when passing.
    Item,
}

/// Artifact types (the shield icons).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardType {
    /// Dragon.
    Dragon,
    /// Creature.
    Creature,
}

/// Victory points: a fixed part plus optionally a number per essence sitting on
/// the card.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Vp {
    /// Points always counted.
    pub base:        u32,
    /// `(points, essence)`: that many points for each such essence on the card.
    pub per_essence: Option<(u32, Essence)>,
}

/// What a cost asks for besides essences.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which {
    /// The card carrying the power.
    This,
    /// Another artifact of yours.
    Other,
    /// Any artifact of yours, possibly this one.
    Own,
}

/// One part of a power's cost.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CostPart {
    /// Turn this component sideways.
    Turn,
    /// Pay these essences to the supply.
    Pay(Amount),
    /// Pay one of these.
    PayOneOf(Vec<Amount>),
    /// Turn another component of that type.
    TurnOther(CardType),
    /// Destroy an artifact.
    Destroy(Which),
    /// Discard cards from hand.
    Discard(u32),
    /// Take these essences off this card (back to the supply).
    TakeFromCard(Amount),
}

/// A deck an effect looks at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Deck {
    /// Your artifact deck.
    Artifact,
    /// The monument deck.
    Monument,
}

/// What a straighten effect may straighten.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Straighten {
    /// Any of your turned components.
    Any,
    /// Only Creature artifacts.
    Creature,
    /// The card carrying the power.
    This,
}

/// What a rival may do to ignore Life loss from a power.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RivalReact {
    /// Pay these essences.
    Pay(Amount),
    /// Discard a card from hand.
    DiscardCard,
    /// Destroy one of their own artifacts.
    DestroyArtifact,
}

/// Whose cost an effect copies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CostSource {
    /// The artifact discarded as part of the cost.
    Discarded,
    /// The artifact destroyed as part of the cost.
    Destroyed,
}

/// Where a placed artifact comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Zone {
    /// Your hand.
    Hand,
    /// Your discard pile.
    Discard,
}

/// The price of placing an artifact through an effect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Price {
    /// Free (cost 0).
    Free,
    /// Normal cost minus this much.
    Discount(Amount),
}

/// Where an essence paid as a cost ends up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PutOn {
    /// On the card carrying the power.
    This,
    /// On any of your components.
    AnyComponent,
}

/// One part of a power's effect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EffectPart {
    /// Take essences from the supply into your pool, or onto this card (`@`).
    Gain {
        /// What to take.
        options: Options,
        /// Put on this card instead of into the pool.
        on_card: bool,
    },
    /// Draw cards.
    Draw(u32),
    /// Discard cards.
    Discard(u32),
    /// Look at the top cards of a deck, reorder and put back.
    Peek {
        /// How many cards.
        count: u32,
        /// The decks you may choose from.
        decks: Vec<Deck>,
    },
    /// Straighten a turned component.
    Straighten(Straighten),
    /// Ignore a Life loss (a React effect).
    IgnoreLifeLoss,
    /// All rivals lose Life; rivals may ignore it by the given reaction.
    RivalsLoseLife {
        /// Life lost.
        amount: u32,
        /// How a rival may ignore it, besides their own reactions.
        unless: Option<RivalReact>,
    },
    /// All rivals gain essences (passed rivals too).
    RivalsGain(Amount),
    /// Gain `gain` equal to the number of `counted` essences in a rival's pool.
    GainPerRival {
        /// The essence gained.
        gain:    Essence,
        /// The essence counted.
        counted: Essence,
    },
    /// Gain any mix of essences equal to a card's placement cost plus a bonus.
    GainCost {
        /// Whose cost.
        of:      CostSource,
        /// Added to the number of essences in the cost.
        bonus:   u32,
        /// Essences that may not be taken.
        exclude: Vec<Essence>,
    },
    /// Place an artifact.
    Place {
        /// Only Dragons may be placed; `None` means any artifact.
        only:  Option<CardType>,
        /// Where it comes from.
        from:  Zone,
        /// What it costs.
        price: Price,
    },
    /// Put the essence paid in the cost on a component.
    PutPaid(PutOn),
    /// Check victory right now.
    VictoryCheck,
    /// Gain victory points.
    Vp {
        /// Points.
        points:    u32,
        /// Only counted for the current victory check.
        temporary: bool,
    },
}

/// What a React power reacts to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    /// A rival's Life loss effect.
    LifeLoss,
    /// A Dragon's power.
    DragonPower,
    /// A victory check.
    VictoryCheck,
}

/// What a discount applies to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiscountOn {
    /// Placing any artifact.
    Artifacts,
    /// Placing Dragons only.
    Dragons,
}

/// An ability, a power or a reaction of a card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ability {
    /// Collect step: take these essences.
    Collect(Options),
    /// Collect step (Vault): if this essence is left on the card, gain these.
    CollectIfLeft {
        /// The essence that must stay.
        essence: Essence,
        /// What you gain.
        gain:    Options,
    },
    /// Collect step (Windup Man): if essences are left on the card, add this
    /// many of each kind present.
    CollectGrow {
        /// Added per kind.
        per_kind: u32,
    },
    /// Collect step (Cursed Forge): pay this (the curse) or turn the card.
    CollectCurse {
        /// The price of the curse.
        pay: Amount,
    },
    /// When the monument is claimed.
    OnClaim(Options),
    /// A placement discount.
    Discount {
        /// What it applies to.
        on:     DiscountOn,
        /// How much is saved.
        amount: Amount,
    },
    /// A power: pay the cost, apply the effect; an action.
    Power {
        /// What to pay.
        cost:   Vec<CostPart>,
        /// What happens.
        effect: Vec<EffectPart>,
    },
    /// A power used out of turn.
    React {
        /// What it reacts to.
        trigger: Trigger,
        /// What to pay.
        cost:    Vec<CostPart>,
        /// What happens.
        effect:  Vec<EffectPart>,
    },
}

/// A card as written in the card file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardSpec {
    /// The family.
    pub kind:      CardKind,
    /// The printed (German) name, unique among all cards.
    pub name:      String,
    /// The English name, when the rulebook names the card.
    pub english:   Option<String>,
    /// Placement cost; `None` for mages and magic items, `Some(vec![])` for
    /// cost 0.
    pub cost:      Option<Vec<Amount>>,
    /// Victory points.
    pub vp:        Vp,
    /// Dragon / Creature icons.
    pub types:     Vec<CardType>,
    /// Abilities, powers and reactions in printed order.
    pub abilities: Vec<Ability>,
}

impl CardSpec {
    /// Whether the card has the type.
    pub fn is(&self, ty: CardType) -> bool {
        self.types.contains(&ty)
    }
}
