//! Parser for the card file (see the header of `data/cards.ra` for the
//! notation).

use std::{collections::HashSet, error::Error, fmt};

use crate::spec::{
    Ability,
    Amount,
    CardKind,
    CardSpec,
    CardType,
    CostPart,
    CostSource,
    Count,
    Deck,
    DiscountOn,
    EffectPart,
    Essence,
    Options,
    Price,
    PutOn,
    RivalReact,
    Selector,
    Straighten,
    Trigger,
    Vp,
    Which,
    Zone,
};

/// A problem in the card file, with the 1-based line it was found on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    /// Line number.
    pub line:    usize,
    /// What is wrong.
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl Error for ParseError {}

type Res<T> = Result<T, String>;

/// Parses a whole card file.
pub fn parse(text: &str) -> Result<Vec<CardSpec>, ParseError> {
    let mut cards: Vec<CardSpec> = Vec::new();
    let mut names = HashSet::new();
    for (index, raw) in text.lines().enumerate() {
        let at = |message: String| ParseError {
            line: index + 1,
            message,
        };
        let line = strip_comment(raw);
        if line.trim().is_empty() {
            continue;
        }
        if line.starts_with(char::is_whitespace) {
            let card = cards.last_mut().ok_or_else(|| at("ability before any card".into()))?;
            let ability =
                parse_ability(line.trim()).map_err(|e| at(format!("{}: {e}", card.name)))?;
            card.abilities.push(ability);
        } else {
            let card = parse_header(line).map_err(at)?;
            if !names.insert(card.name.clone()) {
                return Err(at(format!("duplicate card {:?}", card.name)));
            }
            cards.push(card);
        }
    }
    Ok(cards)
}

/// Removes a `#` comment. Card names never contain `#`.
fn strip_comment(line: &str) -> &str {
    line.split_once('#').map_or(line, |(before, _)| before).trim_end()
}

// ---------------------------------------------------------------------------------------------------- header

/// Splits a header into whitespace separated words, keeping quoted text
/// (`en="Sea Serpent"`) together.
fn words(line: &str) -> Res<Vec<String>> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for c in line.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            },
            c => current.push(c),
        }
    }
    if quoted {
        return Err("unterminated quote".into());
    }
    if !current.is_empty() {
        out.push(current);
    }
    Ok(out)
}

fn parse_kind(word: &str) -> Res<CardKind> {
    match word {
        "mage" => Ok(CardKind::Mage),
        "artifact" => Ok(CardKind::Artifact),
        "monument" => Ok(CardKind::Monument),
        "place" => Ok(CardKind::Place),
        "item" => Ok(CardKind::Item),
        other => Err(format!("unknown card kind {other:?}")),
    }
}

fn parse_header(line: &str) -> Res<CardSpec> {
    let words = words(line)?;
    let mut words = words.into_iter();
    let kind = parse_kind(&words.next().ok_or("empty header")?)?;
    let name = words.next().ok_or("card needs a quoted name")?;
    let mut card = CardSpec {
        kind,
        name,
        english: None,
        cost: None,
        vp: Vp::default(),
        types: Vec::new(),
        abilities: Vec::new(),
    };
    for word in words {
        let (key, value) =
            word.split_once('=').ok_or_else(|| format!("expected key=value, got {word:?}"))?;
        match key {
            "en" => card.english = Some(value.to_string()),
            "cost" => card.cost = Some(parse_placement_cost(value)?),
            "vp" => card.vp = parse_vp(value)?,
            "types" => {
                card.types = value
                    .split(',')
                    .map(|t| match t {
                        "dragon" => Ok(CardType::Dragon),
                        "creature" => Ok(CardType::Creature),
                        other => Err(format!("unknown type {other:?}")),
                    })
                    .collect::<Res<_>>()?;
            },
            other => return Err(format!("unknown header key {other:?}")),
        }
    }
    Ok(card)
}

fn parse_placement_cost(text: &str) -> Res<Vec<Amount>> {
    if text == "0" {
        return Ok(Vec::new());
    }
    text.split('+').map(parse_amount).collect()
}

fn parse_vp(text: &str) -> Res<Vp> {
    let mut vp = Vp::default();
    for part in text.split('+') {
        if let Some((points, essence)) = part.split_once('/') {
            let essence = single_symbol(essence)?;
            let points = points.parse().map_err(|_| format!("bad vp {part:?}"))?;
            vp.per_essence = Some((points, essence));
        } else {
            vp.base = part.parse().map_err(|_| format!("bad vp {part:?}"))?;
        }
    }
    Ok(vp)
}

// ----------------------------------------------------------------------------------------------- amounts

fn single_symbol(text: &str) -> Res<Essence> {
    let mut chars = text.chars();
    match (chars.next().and_then(Essence::from_symbol), chars.next()) {
        (Some(e), None) => Ok(e),
        _ => Err(format!("expected one essence symbol, got {text:?}")),
    }
}

/// `[count]symbol` with `count` a number or `N`, `symbol` one of `E L C D G *`;
/// `*` may be followed by `=` (identical) and `-` plus the excluded essence
/// symbols.
fn parse_amount(text: &str) -> Res<Amount> {
    let digits = text.chars().take_while(char::is_ascii_digit).count();
    let (count, rest) = if digits > 0 {
        let n = text[..digits].parse().map_err(|_| format!("bad number in {text:?}"))?;
        (Count::Fixed(n), &text[digits..])
    } else if let Some(rest) = text.strip_prefix('N') {
        (Count::Var, rest)
    } else {
        (Count::Fixed(1), text)
    };
    let mut chars = rest.chars();
    let symbol = chars.next().ok_or_else(|| format!("missing essence in {text:?}"))?;
    let tail = chars.as_str();
    if symbol == '*' {
        let (identical, tail) = tail.strip_prefix('=').map_or((false, tail), |t| (true, t));
        let exclude = match tail.strip_prefix('-') {
            Some(list) => {
                list.chars().map(|c| single_symbol(&c.to_string())).collect::<Res<Vec<_>>>()?
            },
            None if tail.is_empty() => Vec::new(),
            None => return Err(format!("unexpected {tail:?} in {text:?}")),
        };
        Ok(Amount {
            count,
            what: Selector::Any { exclude },
            identical,
        })
    } else {
        if !tail.is_empty() {
            return Err(format!("unexpected {tail:?} in {text:?}"));
        }
        let essence = Essence::from_symbol(symbol)
            .ok_or_else(|| format!("unknown symbol {symbol:?} in {text:?}"))?;
        Ok(Amount {
            count,
            what: Selector::Essence(essence),
            identical: false,
        })
    }
}

fn fixed(amount: &Amount) -> Res<u32> {
    match amount.count {
        Count::Fixed(n) => Ok(n),
        Count::Var => Err("a fixed number is required here".into()),
    }
}

fn parse_options(text: &str) -> Res<Options> {
    text.split('/')
        .map(|group| group.split('+').map(parse_amount).collect::<Res<Vec<_>>>())
        .collect::<Res<Vec<_>>>()
        .map(Options)
}

fn number(text: &str) -> Res<u32> {
    text.parse().map_err(|_| format!("expected a number, got {text:?}"))
}

// ---------------------------------------------------------------------------------------------- abilities

fn parse_ability(line: &str) -> Res<Ability> {
    let (directive, rest) =
        line.split_once(char::is_whitespace).map_or((line, ""), |(d, r)| (d, r.trim()));
    match directive {
        "collect" => Ok(Ability::Collect(parse_options(rest)?)),
        "collect-left" => {
            let (essence, gain) = rest.split_once('>').ok_or("expected `<essence> > <options>`")?;
            Ok(Ability::CollectIfLeft {
                essence: single_symbol(essence.trim())?,
                gain:    parse_options(gain.trim())?,
            })
        },
        "collect-grow" => Ok(Ability::CollectGrow {
            per_kind: number(rest)?,
        }),
        "collect-curse" => Ok(Ability::CollectCurse {
            pay: parse_amount(rest)?,
        }),
        "onclaim" => Ok(Ability::OnClaim(parse_options(rest)?)),
        "discount" => {
            let (on, amount) = rest
                .split_once(char::is_whitespace)
                .ok_or("expected `<artifact|dragon> <amount>`")?;
            let on = match on {
                "artifact" => DiscountOn::Artifacts,
                "dragon" => DiscountOn::Dragons,
                other => return Err(format!("unknown discount target {other:?}")),
            };
            Ok(Ability::Discount {
                on,
                amount: parse_amount(amount.trim())?,
            })
        },
        "power" => {
            let (cost, effect) = parse_cost_effect(rest)?;
            Ok(Ability::Power { cost, effect })
        },
        "react" => {
            let (trigger, rest) =
                rest.split_once(char::is_whitespace).ok_or("expected a trigger")?;
            let trigger = match trigger {
                "lifeloss" => Trigger::LifeLoss,
                "dragon-power" => Trigger::DragonPower,
                "victory-check" => Trigger::VictoryCheck,
                other => return Err(format!("unknown trigger {other:?}")),
            };
            let (cost, effect) = parse_cost_effect(rest.trim())?;
            Ok(Ability::React {
                trigger,
                cost,
                effect,
            })
        },
        other => Err(format!("unknown directive {other:?}")),
    }
}

fn parse_cost_effect(text: &str) -> Res<(Vec<CostPart>, Vec<EffectPart>)> {
    let (cost, effect) = text.split_once('>').ok_or("expected `<cost> > <effect>`")?;
    let cost = cost.trim().split('+').map(parse_cost_part).collect::<Res<_>>()?;
    let effect = effect.trim().split('+').map(parse_effect_part).collect::<Res<_>>()?;
    Ok((cost, effect))
}

fn parse_type(text: &str) -> Res<CardType> {
    match text {
        "dragon" => Ok(CardType::Dragon),
        "creature" => Ok(CardType::Creature),
        other => Err(format!("unknown type {other:?}")),
    }
}

fn parse_cost_part(part: &str) -> Res<CostPart> {
    if part == "T" {
        return Ok(CostPart::Turn);
    }
    if let Some((head, tail)) = part.split_once(':') {
        return match head {
            "turn" => Ok(CostPart::TurnOther(parse_type(tail)?)),
            "destroy" => Ok(CostPart::Destroy(match tail {
                "self" => Which::This,
                "other" => Which::Other,
                "own" => Which::Own,
                other => return Err(format!("unknown destroy target {other:?}")),
            })),
            "discard" => Ok(CostPart::Discard(number(tail)?)),
            "take" => Ok(CostPart::TakeFromCard(parse_amount(tail)?)),
            other => Err(format!("unknown cost part {other:?}")),
        };
    }
    if part.contains('/') {
        return part.split('/').map(parse_amount).collect::<Res<_>>().map(CostPart::PayOneOf);
    }
    parse_amount(part).map(CostPart::Pay)
}

fn parse_decks(text: &str) -> Res<Vec<Deck>> {
    text.split('|')
        .map(|d| match d {
            "artifact" => Ok(Deck::Artifact),
            "monument" => Ok(Deck::Monument),
            other => Err(format!("unknown deck {other:?}")),
        })
        .collect()
}

fn parse_zone(text: &str) -> Res<Zone> {
    match text {
        "hand" => Ok(Zone::Hand),
        "discard" => Ok(Zone::Discard),
        other => Err(format!("unknown zone {other:?}")),
    }
}

fn exclusions(amount: &Amount) -> Res<Vec<Essence>> {
    match &amount.what {
        Selector::Any { exclude } => Ok(exclude.clone()),
        Selector::Essence(_) => Err("an any-essence amount (`*`) is required here".into()),
    }
}

fn parse_effect_part(part: &str) -> Res<EffectPart> {
    match part {
        "ignore" => return Ok(EffectPart::IgnoreLifeLoss),
        "victory-check" => return Ok(EffectPart::VictoryCheck),
        _ => {},
    }
    // The rivals' reaction (`~pay:1D`) itself contains colons, so take it off
    // first.
    let (part, react) = part.split_once('~').map_or((part, None), |(p, r)| (p, Some(r)));
    if react.is_some() && !part.starts_with("rivals:lose:") {
        return Err(format!(
            "only Life loss can carry a rival reaction: {part:?}"
        ));
    }
    let fields: Vec<&str> = part.split(':').collect();
    match fields.as_slice() {
        ["draw", n] => Ok(EffectPart::Draw(number(n)?)),
        ["discard", n] => Ok(EffectPart::Discard(number(n)?)),
        ["peek", n, decks] => Ok(EffectPart::Peek {
            count: number(n)?,
            decks: parse_decks(decks)?,
        }),
        ["straighten", what] => Ok(EffectPart::Straighten(match *what {
            "any" => Straighten::Any,
            "creature" => Straighten::Creature,
            "self" => Straighten::This,
            other => return Err(format!("unknown straighten target {other:?}")),
        })),
        ["rivals", "lose", amount] => {
            let amount = parse_amount(amount)?;
            if amount.what != Selector::Essence(Essence::Life) {
                return Err("Life loss must be written in Life (L)".into());
            }
            let unless = react.map(parse_rival_react).transpose()?;
            Ok(EffectPart::RivalsLoseLife {
                amount: fixed(&amount)?,
                unless,
            })
        },
        ["rivals", "gain", amount] => Ok(EffectPart::RivalsGain(parse_amount(amount)?)),
        ["gain-per-rival", gain, counted] => Ok(EffectPart::GainPerRival {
            gain:    single_symbol(gain)?,
            counted: single_symbol(counted)?,
        }),
        ["gain-cost", of, bonus, amount] => Ok(EffectPart::GainCost {
            of:      match *of {
                "discarded" => CostSource::Discarded,
                "destroyed" => CostSource::Destroyed,
                other => return Err(format!("unknown cost source {other:?}")),
            },
            bonus:   number(bonus)?,
            exclude: exclusions(&parse_amount(amount)?)?,
        }),
        ["place", what, from, price] => Ok(EffectPart::Place {
            only:  match *what {
                "artifact" => None,
                "dragon" => Some(CardType::Dragon),
                other => return Err(format!("unknown place target {other:?}")),
            },
            from:  parse_zone(from)?,
            price: match price.strip_prefix("discount=") {
                Some(amount) => Price::Discount(parse_amount(amount)?),
                None if *price == "free" => Price::Free,
                None => return Err(format!("unknown price {price:?}")),
            },
        }),
        ["put-paid", on] => Ok(EffectPart::PutPaid(match *on {
            "self" => PutOn::This,
            "any" => PutOn::AnyComponent,
            other => return Err(format!("unknown put-paid target {other:?}")),
        })),
        ["vp", points, "temp"] => Ok(EffectPart::Vp {
            points:    number(points)?,
            temporary: true,
        }),
        [single] => {
            let (body, on_card) = single.strip_suffix('@').map_or((*single, false), |b| (b, true));
            Ok(EffectPart::Gain {
                options: parse_options(body)?,
                on_card,
            })
        },
        _ => Err(format!("cannot read effect {part:?}")),
    }
}

fn parse_rival_react(text: &str) -> Res<RivalReact> {
    match text {
        "discard" => Ok(RivalReact::DiscardCard),
        "destroy" => Ok(RivalReact::DestroyArtifact),
        other => match other.strip_prefix("pay:") {
            Some(amount) => Ok(RivalReact::Pay(parse_amount(amount)?)),
            None => Err(format!("unknown rival reaction {other:?}")),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const E: Essence = Essence::Elan;
    const G: Essence = Essence::Gold;

    #[test]
    fn amounts_use_the_rulebook_symbols() {
        assert_eq!(parse_amount("E").unwrap(), Amount::of(1, E));
        assert_eq!(parse_amount("3G").unwrap(), Amount::of(3, G));
        assert_eq!(
            parse_amount("2*").unwrap(),
            Amount {
                count:     Count::Fixed(2),
                what:      Selector::Any { exclude: vec![] },
                identical: false,
            }
        );
        assert_eq!(
            parse_amount("3*-GD").unwrap(),
            Amount {
                count:     Count::Fixed(3),
                what:      Selector::Any {
                    exclude: vec![G, Essence::Death],
                },
                identical: false,
            }
        );
        assert_eq!(
            parse_amount("N*=").unwrap(),
            Amount {
                count:     Count::Var,
                what:      Selector::Any { exclude: vec![] },
                identical: true,
            }
        );
        assert_eq!(
            parse_amount("NG").unwrap(),
            Amount {
                count:     Count::Var,
                what:      Selector::Essence(G),
                identical: false,
            }
        );
    }

    #[test]
    fn amounts_round_trip_through_display() {
        for text in ["E", "3G", "2*", "*-G", "3*-GL", "N*=", "N*=-G", "NG", "6C"] {
            assert_eq!(parse_amount(text).unwrap().to_string(), text);
        }
    }

    #[test]
    fn bad_amounts_are_rejected() {
        for text in ["", "3", "X", "2Z", "EE", "*x", "G-E", "3*-Q"] {
            assert!(parse_amount(text).is_err(), "{text:?} should not parse");
        }
    }

    #[test]
    fn options_distinguish_choice_from_conjunction() {
        let either = parse_options("C/L").unwrap();
        assert_eq!(either.0.len(), 2);
        let both = parse_options("D+E").unwrap();
        assert_eq!((both.0.len(), both.0[0].len()), (1, 2));
    }

    #[test]
    fn headers_read_quotes_keys_and_vp() {
        let cards = parse(
            "artifact \"Sea Serpent X\" en=\"The Serpent\" cost=6C+3L vp=1 types=dragon,creature\n",
        )
        .unwrap();
        let c = &cards[0];
        assert_eq!(
            (c.name.as_str(), c.english.as_deref()),
            ("Sea Serpent X", Some("The Serpent"))
        );
        assert_eq!(
            c.cost,
            Some(vec![
                Amount::of(6, Essence::Calm),
                Amount::of(3, Essence::Life)
            ])
        );
        assert_eq!(
            c.vp,
            Vp {
                base:        1,
                per_essence: None,
            }
        );
        assert!(c.is(CardType::Dragon) && c.is(CardType::Creature));
        let place = parse("place \"P\" vp=2+1/L\n").unwrap();
        assert_eq!(
            place[0].vp,
            Vp {
                base:        2,
                per_essence: Some((1, Essence::Life)),
            }
        );
        let free = parse("artifact \"F\" cost=0\n").unwrap();
        assert_eq!(free[0].cost, Some(vec![]));
    }

    #[test]
    fn powers_split_cost_and_effect() {
        let cards = parse("mage \"M\"\n  power T+2* > 3*-G\n").unwrap();
        let Ability::Power { cost, effect } = &cards[0].abilities[0] else {
            panic!("expected a power");
        };
        assert_eq!(cost.len(), 2);
        assert_eq!(cost[0], CostPart::Turn);
        assert!(matches!(
            effect.as_slice(),
            [EffectPart::Gain { on_card: false, .. }]
        ));
    }

    #[test]
    fn effects_cover_the_special_forms() {
        let ok = |text: &str| parse_effect_part(text).unwrap();
        assert_eq!(ok("draw:3"), EffectPart::Draw(3));
        assert_eq!(
            ok("straighten:creature"),
            EffectPart::Straighten(Straighten::Creature)
        );
        assert_eq!(
            ok("rivals:lose:2L~pay:1D"),
            EffectPart::RivalsLoseLife {
                amount: 2,
                unless: Some(RivalReact::Pay(Amount::of(1, Essence::Death))),
            }
        );
        assert_eq!(
            ok("gain-per-rival:E:D"),
            EffectPart::GainPerRival {
                gain:    E,
                counted: Essence::Death,
            }
        );
        assert_eq!(
            ok("place:dragon:hand:free"),
            EffectPart::Place {
                only:  Some(CardType::Dragon),
                from:  Zone::Hand,
                price: Price::Free,
            }
        );
        assert_eq!(
            ok("vp:3:temp"),
            EffectPart::Vp {
                points:    3,
                temporary: true,
            }
        );
        assert!(matches!(ok("3D@"), EffectPart::Gain { on_card: true, .. }));
    }

    #[test]
    fn errors_carry_the_line_number_and_card() {
        let err = parse("# comment\nmage \"M\"\n  power T > bogus:1\n").unwrap_err();
        assert_eq!(err.line, 3);
        assert!(err.message.contains("M:"), "{err}");
        assert_eq!(parse("  power T > ignore\n").unwrap_err().line, 1);
        assert!(parse("wizard \"W\"\n").unwrap_err().message.contains("unknown card kind"));
        assert!(parse("mage \"A\"\nmage \"A\"\n").unwrap_err().message.contains("duplicate"));
        assert!(parse("mage \"A\n").unwrap_err().message.contains("quote"));
        assert!(
            parse("mage \"A\" color=red\n").unwrap_err().message.contains("unknown header key")
        );
        assert!(parse("mage \"A\"\n  power T ignore\n").unwrap_err().message.contains("expected"));
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let cards = parse("# header\n\nmage \"M\"   # trailing\n\n  collect E # why\n").unwrap();
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].abilities.len(), 1);
    }

    #[test]
    fn life_loss_must_be_written_in_life() {
        assert!(parse_effect_part("rivals:lose:2E").is_err());
    }
}
