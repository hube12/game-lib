# gamelib

A headless, deterministic rules engine for board games, written in Rust. No UI, no networking, no game code in the
core. A game is fully determined by **a seed plus its action log**, so any game can be replayed, tested, fuzzed or
streamed to a client.

Status: the engine, a toy deckbuilder and **the Res Arcana base game are playable and tested**: all 73 base cards
compile into engine data, `ResArcanaRules` sets up a table and plays whole games (setup, the five phases, pass
procedure, scoring), and random games replay identically. See [Status](#status). `plan.md` is the detailed design and progress log.

## Design principle

Few generic primitives, composed by data:

- **entities in zones** (cards, tokens, dice; deck, hand, tableau, market row, ...)
- **events** (everything that changes state emits one)
- **triggered effects** (abilities react to events, cost nothing; activated abilities and reactions pay a cost)
- **modifiers** (temporary or permanent rule changes)
- a **phase / turn state machine**
- **deterministic replay** (seeded RNG inside the state, everything `serde`-serializable and `PartialEq`)

### The architecture rule (read this before touching `src/`)

**The core crate names no game concept.** There is no `Life`, `Dragon`, `essence`, `victory check`, `Gold` or `deck of
mages` anywhere in `src/`. A game crate gives meaning through data:

| Engine primitive                 | How a game uses it                                                                         |
| -------------------------------- | ------------------------------------------------------------------------------------------ |
| `ResourceId(u16)`                | `COIN` in the toy game; Elan/Life/Calm/Death/Gold in Res Arcana                            |
| `ZoneKind(u16)`                  | `DECK`, `HAND`, `DISCARD`, `ROW`, ... constants defined by the game                        |
| tags (`CardDef.tags: Vec<String>`) | card types such as "dragon", "creature"; filters and discounts match on them             |
| `ReactKind(u16)`                 | the game's reaction ids (for example "life loss"); the engine gives them no meaning        |
| `CounterId(u16)`, `PhaseId(u16)` | score tracks, phases                                                                       |
| generic effects                  | compositions such as `Effect::Affect`, `Effect::Lose`, `Effect::ReactWindow`, `Effect::CheckWin` |

If a mechanic needs a new engine feature, add a **generic** primitive (never a game-named one) and compose it in the
game crate. Example from the plan: a victory check is `Sequence[ForEachPlayer(ReactWindow{CHECK, Ignore, repeat}),
CheckWin, ResetCounters{temp}]`, and a "lose 2 Life, rivals may react" power is `Affect{Rivals, Defense{react:
[LIFE_LOSS], ..}, Lose{LIFE, 2, shortfall: 2}}`. Both are built in the game crate, not in the core.

## Repository map

Cargo workspace (members: `.`, `cli`, `games/toy`, `games/res-arcana`):

| Path                | Crate                | What it is                                                                                  |
| ------------------- | -------------------- | ------------------------------------------------------------------------------------------- |
| `src/`              | `gamelib`            | The engine (the core crate). No game concept. Modules below.                                |
| `games/toy/`        | `gamelib-toy`        | A small deckbuilder (`ToyRules`) that exercises the engine; the reference for writing a game. |
| `games/res-arcana/` | `gamelib-res-arcana` | Res Arcana base game: `data/cards.ra` (the 73 cards as text), `parse.rs`/`spec.rs` (parser and typed `CardSpec`), `compile.rs` (card specs to `CardDef`s, plus the game's id constants), `rules.rs` (`ResArcanaRules`: setup, phases, pass, scoring), `tests/` (card data, compile, a played round, random games). |
| `cli/`              | `gamelib-cli`        | Headless runner: bots play whole games (`--game toy|res-arcana`, `--bot random|heuristic`), then every game is replayed and compared. |
| `fuzz/`             | (standalone)         | `cargo-fuzz` (LibFuzzer) crate. Currently only a placeholder target (`fuzz_dummy`).          |
| `afl-fuzz/`         | (standalone)         | AFL crate with `run.sh`. Currently only a placeholder harness. Both fuzz crates have their own `[workspace]` and are outside the workspace on purpose. |
| `benches/`          | (root crate)         | Criterion benchmark skeleton (`bench1.rs`), no real benchmarks yet.                          |
| `examples/`         | (root crate)         | Empty `run.rs`.                                                                              |
| `scripts/`          | -                    | `run-local-ci.sh` (full CI sequence), `cross_test.sh`, `build.sh`/`run.sh` (run as another user). |
| `game-rules/`       | -                    | Local rulebooks and card photos. **Gitignored on purpose**; you will not find it in a clone. |
| `plan.md`           | -                    | Design, decisions, status and pitfalls. Read its Status and Workflow sections first.         |

Modules of the core crate (`src/`):

| Module                | Contents                                                                                                    |
| --------------------- | ----------------------------------------------------------------------------------------------------------- |
| `id`                  | Typed id newtypes: `PlayerId`, `EntityId`, `DefId`, `ZoneId`, `ZoneKind`, `ResourceId`, `CounterId`, `ReactKind`, `PhaseId` |
| `rng`                 | `Rng`: ChaCha8, serializable, lives inside the state                                                        |
| `resource`            | `Pool`, `Bank` (a pool whose entries may be infinite), `Supply`, `transfer`                                 |
| `counter`             | `Counter` (value, min, max, owner)                                                                          |
| `zone`                | `Zone`, `Owner`, `Order`, `Visibility`, `Pos`                                                               |
| `card`                | `CardDef` (static), `Entity` (instance state), `Discount`                                                   |
| `cost`, `pay`         | `Cost`, `Payment`, and `pay::enumerate` (all concrete ways to pay)                                          |
| `effect`              | `Effect`, `Ability`, `Trigger`, `Condition`, `Limit`, `Ctx`, `PendingDecision`, `Task`                      |
| `event`, `modifier`   | `Event`, `EventKind`; `Modifier`, `RuleChange`, `Duration`                                                  |
| `market`              | `Market`, `MarketPolicy`, `Refill`                                                                          |
| `flow`                | `Flow`, `Phase`, `TurnOrder`, `RoundEndPolicy`, `GameEndPolicy`                                             |
| `state`               | `GameState`: everything that changes, plus helpers (draw, shuffle, move, gain, pay)                         |
| `view`                | `PlayerView`: what one player may know                                                                      |
| `engine`              | `Engine`, `Action`, `GameRules`, `RuleError`, `standard_legal_actions`; the resolution loop                 |

## Core concepts

All snippets use the real API; the toy game (`games/toy/src/lib.rs`) is the complete worked example.

### Zones, owners, teams

A `Zone` holds entities. It has a `ZoneKind` (a role defined by the game), an `Owner`, an `Order`
(`Stack` / `Row` / `Unordered`) and a `Visibility` (`Public` / `Owner` / `Hidden`). Per owner there is **at most one
zone per kind**, so effects name zones by kind plus owner (`OwnerRef::Controller`, `Shared`, `Player(p)`,
`Players(team)`). `Owner::Players(vec![..])` is a team: lookups compare the list as written, so keep it ascending.
`abilities_active` says whether cards in the zone have live abilities (tableau yes, deck no); `recycle_from` reshuffles
another zone into this one when drawing from it empty.

```rust
let deck = s.add_zone(DECK, Owner::Player(p), Order::Stack, Visibility::Hidden);
s.zone_mut(deck)?.recycle_from = Some(DISCARD);
```

### Entities and card definitions

A `CardDef` is shared by all copies of a card; an `Entity` is one instance (`def`, `zone`, `face_up`, `exhausted`,
and a `pool` of resources stored on the card). A def has `tags`, an optional `cost` (to buy or place it), `on_gain`
(resolved when bought), `abilities` and passive `discounts`.

```rust
let mut silver = CardDef { id: cards::SILVER, name: "Silver".into(), tags: vec![],
    cost: Some(Cost::Resources(vec![(COIN, 3)])), on_gain: None, abilities: vec![], discounts: vec![] };
silver.abilities = vec![play(2)]; // an Ability, see below
s.create_entity(cards::SILVER, supply_zone, false)?; // def, zone, face_up
```

### Resources, pools, the bank

A `Pool` maps `ResourceId` to a `Supply` (`Finite(n)` or `Infinite`). Players and cards hold pools; the shared **bank**
(`state.bank`) is a pool too. Gains come out of the bank, payments go back to it (`s.bank.set_infinite(COIN)` for an
unlimited supply, `set(res, n)` for a finite one, in which case gains are capped to what is left). `Counter`s (score,
tracks) are separate from resources, have bounds and an owner.

### Costs and payments

A `Cost` is a description: `Resources`, `Or`, `All`, `Exhaust` (turn the card), `Any{n, exclude, identical}` (any mix
from the pool), `TakeFromCard`, `TurnOther{tag}`, `Destroy{which, tag}`, `Discard{from, n}`. `pay::enumerate(state,
player, source, cost)` expands it into every concrete `Payment` the player can **afford now**, deduplicated and in a
deterministic order (capped at 4000). A `Payment` records what is paid (`resources`, `exhaust`, `from_card`, `turned`,
`destroyed`, `discarded`, `x`).

Actions carry an **index** into that list:

```rust
Action::Buy { entity, pay }            // pay = index into the affordable payments for that card
Action::Activate { entity, ability, pay }
Action::Place { entity, pay }          // pay = index into pay::place_payments (cost minus discounts)
```

Because the index means nothing without the state, `legal_actions` and `apply` must enumerate against the same state
and the enumeration order must never change between the two. Placement discounts (`CardDef.discounts`) are cumulative
buckets applied by `pay::enumerate_discounted`; costs never drop below zero.

### Effects and the resolution stack

An `Effect` is plain serializable data (no function pointers): `Gain`, `Pay`, `Draw`, `Move`, `MoveAll`,
`ModifyCounter`, `AddModifier`, `Exhaust`/`Ready`, `Destroy`, `Sequence`, `Choose`, `ForEachPlayer`, `Pick`, and more
(see `src/effect.rs`). Everything goes through one loop in `src/engine.rs`:

1. An action pushes `Task`s on `state.stack`; every state change queues an `Event`.
2. `settle()` drains queued events first (modifiers expire, limits reset, abilities trigger, the win check runs), then
   pops one task. Triggered effects go on top, so they resolve before whatever was already waiting.
3. The loop stops when the stack is empty or a `PendingDecision` needs input.
4. `apply` works on a backup: on any error the whole state, events and log roll back. A rules loop ends in
   `RuleError::Runaway`.

An `Ability` is `{trigger, condition, cost, effect, limit}`. From the toy game, a card that gives coins when played:

```rust
Ability {
    trigger: Trigger::Activated, condition: Condition::Always, cost: None, limit: Limit::None,
    effect: Effect::Sequence(vec![
        Effect::Gain { to: Target::Controller, res: COIN, n: 2 },
        Effect::Move { entity: Target::Source, to: PLAYED, owner: OwnerRef::Controller },
    ]),
}
```

`Target` and `OwnerRef` are relative to the ability (`Controller`, `Source`, `Card`, ...), so one definition works for
every player. `Ctx.bound` carries values bound while resolving (the picked card, the variable `x` chosen while paying,
the single resource paid).

### Events and triggers

`state.emit(Event::..)` is called by every state change (`EntityMoved`, `ResourceGained`, `TurnStarted`,
`PhaseStarted`, `Passed`, ...). A `Trigger` says what makes an ability happen:

- `Trigger::Activated`: the player uses it as an action and pays the cost.
- `Trigger::OnEvent(EventKind)`: fires by itself, no cost. `Condition::InPhase(phase)` restricts it, which is how a
  "collect" ability hooks one phase (plain `OnEvent(PhaseStarted)` would fire for every phase).
- `Trigger::React(ReactKind)`: not event-driven; offered by an effect that opens a reaction window.

`Limit` (`OncePerTurn`, `OncePerRound`, `OncePerBetweenTurns`) is enforced through recorded uses that reset on events.

### Decisions

`Effect::Choose(options)` stops the engine with a `PendingDecision {players, options, answers, ..}`. Each option is
itself an effect, resolved with the answering player as controller. Players answer with `Action::Decide(i)`. Several
players can decide at the same time (drafts, secret bids); the chosen options resolve once everyone answered, so a
multi-player decision needs controller-relative options. While a decision is pending, `legal_actions` offers only
`Decide(0..n)` to players who still owe an answer.

### Reactions

Generic building blocks, no game names: `Effect::Affect{who, defense, effect}` aims `effect` at every affected player
in turn order; each one gets a decision made of the effect itself, one `Use` option per affordable React ability of the
kinds in `defense.react`, and an optional `defense.escape` (`Pay`, `DiscardCard`, `Destroy(tag)`). `Effect::Lose{res, n,
shortfall}` loses `n`, and for each unit missing `shortfall` other resources of the player's choice.
`Effect::ReactWindow{kind, default, repeat}` offers the controller their own React abilities. `Effect::Use` pays and
resolves an ability without counting as an action.

### Modifiers

A `Modifier {target: Subject, rule: RuleChange, duration: Duration}` changes rules for a player or a card:
`RuleChange::Forbid(ActionKind::Buy | Activate)` or `CostDelta{res, delta}`. Durations: `ThisTurn`, `NextTurnOf(p)`,
`UntilRoundEnd`, `UntilEvent(kind)`, `Permanent`. Add one with `Effect::AddModifier`; the engine expires them from
events. Note: `CostDelta` applies to buying and activating, not to placement (use `CardDef.discounts` for that).

### Markets

A `Market {zone, source, size, dest, policy}` is a shared zone players buy from, refilled from a source zone. The
policy chooses `top_only`, when to `Refill` (`Immediate`, `EndOfTurn`, `Manual`), where removed cards go, and whether
the top card of the source zone can be bought blind (`source_buyable`).
`standard_legal_actions` turns each buyable card and affordable payment into an `Action::Buy`.

### Flow: phases, turns, rounds, game end

`Flow {phases, turn_order, round_end, actions_per_turn, game_end, ..}`. A `Phase {id, interactive}` is either played in
turns or resolves automatically (its `PhaseStarted` event is what abilities hook). `TurnOrder` tracks players and
passes (`TurnOrderPolicy::{Clockwise, PassOrder, FirstPasserStarts}`); `RoundEndPolicy::{FirstPass, AllPassed}` says when an interactive
phase ends; `actions_per_turn: Some(1)` ends the turn automatically. `Flow::active_player()` is `None` outside
interactive phases. `GameEndPolicy {counter, target, when, tiebreak}` is the simple "first to N points" rule; games with
other scoring override `GameRules::winners`.

### Hidden information

`Engine::view(player)` returns a `PlayerView`: zone counts are always visible, entities only when the viewer may see
them (`Visibility`), face-down cards appear with `def: None`, and the options of a pending decision are shown only to a
player who still has to answer. Clients should only ever see a `PlayerView`, never the `GameState`.

### GameRules and the Engine

A game implements `GameRules`:

```rust
pub trait GameRules {
    fn setup(&self, players: &[PlayerId], rng: Rng) -> GameState;                  // required
    fn legal_actions(&self, state: &GameState, player: PlayerId) -> Vec<Action>;   // default: standard_legal_actions
    fn winners(&self, state: &GameState) -> Option<Vec<PlayerId>>;                  // default: the flow's GameEndPolicy
    fn phase_effects(&self, state: &GameState, phase: PhaseId) -> Vec<Effect>;      // default: none; runs when a phase starts
    fn pass_effects(&self, state: &GameState, player: PlayerId) -> Vec<Effect>;     // default: none; runs on Action::Pass
    fn turn_end_effects(&self, state: &GameState, player: PlayerId) -> Vec<Effect>; // default: none
}
```

`setup` must move the given `Rng` into the state, so shuffles during setup and play share one stream; tasks queued by
`setup` (for example a mage draft as a pending decision) run before the first phase starts. `GameState.card_rules`
(`CardRules{hand, tableau, discard, on_discard}`), when set, makes `standard_legal_actions` also offer `Action::Place`
and `Action::Discard` for cards in hand.

```rust
let mut game = Engine::new(ToyRules, 2, 42)?;                  // players 0..2, seed 42, runs to the first decision
let p = game.current_player().unwrap();
let action = game.legal_actions(p)[0].clone();
let events = game.apply(p, action)?;                           // Err(RuleError::Illegal) if not in legal_actions
let view = game.view(p);                                       // PlayerView
let again = Engine::replay(ToyRules, 2, 42, game.log())?;      // same seed + log = identical state
assert_eq!(again.state(), game.state());
```

Known limitation: `legal_actions` is recomputed on every call and `apply` clones the state for rollback. Fine for small
tables; optimize before large games.

## Res Arcana

`games/res-arcana/data/cards.ra` holds the 73 base-game cards (10 mages, 10 monuments, 5 places of power, 8 magic items,
40 artifacts) as text written with the rulebook's symbols. Ids are the printed German names; `en="..."` is given only
where the rulebook names the card. Only one side of each place of power is transcribed, and expansions are out of
scope for now. The header of the file is the authoritative grammar; a summary:

```
mage "Duellant" en="Duelist"
  collect E
  power T+D > G@
artifact "Knochendrache" cost=4D+L vp=1 types=dragon
  power T > rivals:lose:2L~pay:1D
monument "Goldene Statue" en="Golden Statue" cost=4G vp=1
  react victory-check T+3G > vp:3:temp
```

- **Symbols.** `E` Elan, `L` Life, `C` Calm, `D` Death, `G` Gold. `*` any essence (`3*` any mix of 3; `*-G`, `*-GD` no
  Gold / no Gold or Death), `N` a player-chosen number (`N*=` = N identical essences), `T` turn this component, `>` the
  triangle between cost and effect, `@` "on the component" (put the essences on the card). An amount on the cost side is
  paid, on the effect side it is gained. `+` means all of these, `/` choose one.
- **Card header.** `<kind> "<name>" [en="..."] [cost=<amounts joined by +>] [vp=<n> | <n>+<m>/<essence> | <m>/<essence>]
  [types=dragon,creature]`. Kinds: `mage`, `monument`, `place`, `item`, `artifact`. `vp=2+1/L` is 2 points plus 1 per
  Life on the card.
- **Body lines (indented).**
  - `collect <options>`: the collect ability. `collect-left <E> > <options>` (Vault), `collect-grow <n>` (Windup Man),
    `collect-curse <amount>` (Cursed Forge).
  - `onclaim <options>`: effect when a monument is claimed.
  - `discount <artifact|dragon> <amount>`: placement discount (cumulative, a cost never drops below 0).
  - `power <cost> > <effect>`: a power; using one is an action.
  - `react <trigger> <cost> > <effect>`: an out-of-turn power. Triggers: `lifeloss`, `dragon-power`, `victory-check`.
- **Cost parts** (joined by `+`): `T`, an amount, `A/B` (pay one of), `turn:dragon`, `turn:creature`, `destroy:self`,
  `destroy:other`, `destroy:own`, `discard:<n>`, `take:<amount>` (take it off this card).
- **Effect parts** (joined by `+`): `<amount>[/<amount>...][@]`, `draw:<n>`, `discard:<n>`, `peek:<n>:<deck>[|<deck>]`,
  `straighten:any|creature|self`, `ignore`, `rivals:lose:<amount> [~pay:<amount> | ~discard | ~destroy]` (the `~` part
  is the rivals' immediate reaction), `rivals:gain:<amount>`, `gain-per-rival:<gain>:<counted>`,
  `gain-cost:<discarded|destroyed>:<bonus>:<amount>`, `place:<artifact|dragon>:<hand|discard>:<free|discount=<amount>>`,
  `put-paid:<self|any>`, `victory-check`, `vp:<n>:temp`.

`gamelib_res_arcana::load()` parses the embedded file into typed `CardSpec`s (`kind`, `name`, `english`, `cost`, `vp`,
`types`, `abilities`); `parse(text)` does the same for any string. If you add a keyword, add it to `parse.rs` and add a
test in `tests/cards.rs` that quotes the rulebook.

### Draft variant (optional)

`ResArcanaRules::new().with_draft(true)` (CLI: `--draft`; the page: the "Draft" checkbox, sent as the `draft` entry of
`NewGame.options`) replaces the random starting decks. After the mages are dealt, everybody is dealt 4 artifacts, keeps 1
and passes the rest to the left (clockwise), and so on until none is left to pass; then 4 more, passed to the right. The 8
kept artifacts are shuffled into the deck and the first 3 are drawn; mages and magic items are chosen afterwards as in the
normal game. Offering the deck to a rival to cut is left out (shuffling is already random). It is built from three
generic engine effects, `Effect::Simultaneous` (everybody picks at once), `Effect::Rotate` (pass a zone's cards around the
table) and `Effect::Shuffle`; the game owns only the composition (`draft()` in `rules.rs`) and the zones `DRAFT` and
`STOCK`. `tests/draft.rs` checks the passing directions, that everybody ends with 8 different artifacts and 3 in hand,
and that a drafted game replays identically.

### Bot difficulty

The planner bot has a dial from 1 (Novice) to 5 (Expert, full strength and the default; `Difficulty` in `planner.rs`). A lower
setting simulates fewer moves per choice, tries fewer of its own follow-up decisions in full, and sometimes plays the
second or third best move it found (never when the best move wins the game). Measured as 40 games against the Expert
planner (alternating seats): Novice 0 wins, Easy 4, Medium 8, Hard 19. Set it with `planner:level=N` in the CLI (for example
`--bots planner:level=2,planner`), with the "Bots" selector on the page, or with the `difficulty=N` entry of
`NewGame.options`. Values outside 1 to 5 are brought into range.

### How cards become engine data

The core engine never sees `CardSpec`. The **compile step** in the game crate (`games/res-arcana/src/compile.rs`)
turns each `CardSpec` into a `CardDef`, and defines every Res Arcana id as a constant (`ELAN`..`GOLD`, `LIFE_LOSS`,
`VICTORY_CHECK`, the phases `TAKE`..`CLEANUP`, the zone kinds, `VP`/`TEMP_VP`) and the tag names:

- card kind and `types=` become tags (`dragon`, `creature`, ...); `cost=` becomes a `Cost`;
- `onclaim` becomes `on_gain`; `discount` becomes `CardDef.discounts`;
- `collect` becomes an ability with `Condition::InPhase(COLLECT)`; `power` an `Activated` ability;
- `react <trigger>` becomes `Trigger::React(kind)` with the game's `ReactKind` constants;
- `rivals:lose:..~..` becomes an `Effect::Affect` whose `Defense` lists the reaction kinds and the printed escape;
- `victory_check(temp)` composes `ReactWindow` + `CheckWin` + `ResetCounters` exactly as in the architecture rule above.

```rust
let specs = gamelib_res_arcana::load()?;
let compiled = gamelib_res_arcana::compile_all(&specs); // ids follow file order
let id = compiled.id_of("Duelist");                     // by printed or English name
// compiled.defs: the cards that compiled; compiled.failures: the ones that could not (none in the base game)
```

Scoring formulas (victory points) are not compiled; `ResArcanaRules::score` reads them from the specs.

## GUI: `gamelib-gui` and the Res Arcana app

`gui/` (`gamelib-gui`) is the game-agnostic part of a GUI. A game implements `GameUi` (the rules, a `GameDescription`
with names, colors and printed card text, a bot for bot seats, a scoreboard) and gets:

- `Session`: a running game with people and bots at its seats. Bots answer instantly; the session stops when a person
  must act. `snapshot(viewer)` returns everything one screen needs: the board as that player may see it (closed zones
  list no cards), the legal actions as readable labels with costs (`index` is what you send back), the log and scores.
  Actions quote the snapshot's `revision`, so a stale click is refused.
- `Backend` and the JSON protocol (`Request`/`Response`: `describe`, `new_game`, `snapshot`, `act`), which any
  transport can carry: a Tauri command, the dev server's `POST /api`, a test.
- `describe`: turns effects, costs, payments and events into text. It also marks the names it uses (cards, zones, resources, counters, phases) so a page can show them
  quoted and underlined with a popup: cards pop up as the card, the rest with the `help` text the game gives them. It hides the names of cards the viewer cannot see
  (a blind claim from the monument deck says "a hidden card").
- `web`: shared no-build ES modules and CSS (`components.js`: resource chips, cards with their action buttons, zones,
  decision dialog (options about cards show the cards themselves, as a shelf when there are many), click-to-zoom on any card, resources as a number plus a colored circle with their letter (also after every resource name in a label), an "On card" marker for what sits on a card, log, scoreboard; `transport.js` picks Tauri or HTTP; `dom.js`).
- `serve` (feature): a tiny HTTP server for developing a page in an ordinary browser.

`games/res-arcana-gui/` supplies Res Arcana: `ResArcanaUi`, the page (`web/index.html`, `app.js`, `res-arcana.css`), and
the `res-arcana-dev` server. Run it in a browser with `just gui-dev` and open `http://127.0.0.1:8787`.

`games/res-arcana-gui/src-tauri/` is the desktop shell (Tauri 2). It is a standalone package (its own `[workspace]`),
because the webview needs system libraries that the rest of the repo does not: Windows needs the WebView2 runtime
(already part of Windows 11), Linux needs `libwebkit2gtk-4.1-dev` and friends. Install the CLI with
`cargo install tauri-cli --version "^2"` and run `just gui-app`. There is no Node step: the page is plain files, and the
shell's build script copies `gamelib-gui`'s shared modules into `web/gamelib-gui/` (generated, git-ignored).

### Card pictures (rough, local)

Cards show a picture when `games/res-arcana-gui/web/cards/<id>.jpg` exists (`id` = the card's definition id); without
the file a card is drawn as text. `just gui-cards` (`scripts/crop_cards.sh`, needs ImageMagick) cuts a rough picture of
every base card out of your local photos using the rectangles in `games/res-arcana-gui/cards.map`. The photos
(`game-rules/`) and the cut pictures are gitignored on purpose: they are someone else's art, so only the map and the
script are committed. To use proper art later, put files with the same names in `web/cards/`; the page and the desktop
build pick them up (the desktop build embeds whatever is in `web/` when it is built).

## Running things

Prerequisites: Rust via `rustup`. The toolchain is pinned in `rust-toolchain.toml`; formatting needs the **nightly**
rustfmt because `rustfmt.toml` uses unstable options. If `cargo` is not on your `PATH`, `export PATH=$HOME/.cargo/bin:$PATH`.

```sh
cargo test --workspace --all-features                                  # all tests
cargo test -p gamelib engine::                                         # a subset: the engine tests
cargo test -p gamelib-res-arcana                                       # card data tests
cargo clippy --workspace --all-targets --all-features -- -D warnings   # lints, warnings are errors
cargo +nightly fmt --all                                               # format (use -- --check to only verify)
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features   # docs, warnings are errors
```

Play the toy game with random bots; every game is replayed afterwards and the resulting state must equal the original
(exit code is non-zero if any game fails or diverges):

```sh
cargo run -p gamelib-cli --release -- --games 100 --seed 1 --players 2
cargo run -p gamelib-cli --release -- --game res-arcana --bot heuristic --games 20 --players 3
# options: --games N  --seed S  --players P (1..=8)  --max-actions M  --draft (Res Arcana draft variant)
# planner difficulty: --bots planner:level=2,planner  (1 = novice .. 5 = full strength)
```

Replay verification in your own code is the `Engine::replay` snippet above: replay `engine.log()` from the same seed and
compare `state()`.

`just` shortcuts (`cargo install just`): `just tests`, `just test <name>`, `just fmt` (nightly), `just lint`, `just bench`,
`just doc`, `just fullbuild`, `just audit`, `just deny`, `just clean`, `just cross`, `just docker`. `scripts/run-local-ci.sh`
runs the whole CI sequence (it installs cargo-audit, tarpaulin, deny and cross first). A pre-commit hook (rusty-hook)
formats the repository.

Fuzzing (both crates currently contain placeholder targets only; the targets do not drive the engine yet):

- LibFuzzer: `cd fuzz && ./run.sh` (needs nightly and `cargo-fuzz`); add targets as `fuzz/fuzz_targets/fuzz_<name>.rs`.
- AFL: `cd afl-fuzz && ./run.sh` (needs `cargo-afl`); seeds in `afl-fuzz/in-<name>/`, harness in `afl-fuzz/src/main.rs`.
  `AFL_SANITIZE=1` builds with AddressSanitizer (nightly). See [the Rust fuzz book](https://rust-fuzz.github.io/book/).

## Contributing rules

- **Nightly rustfmt**: run `cargo +nightly fmt --all` before you hand work in. Note that rustfmt can drop the trailing
  `;` after the last statement of a closure, which then trips clippy's `semicolon_if_nothing_returned`; add it back.
- **Clippy with `-D warnings`**, all targets and features. Crates use `#![deny(missing_docs)]` and pedantic/nursery/cargo
  lints, so every public item needs a doc comment. Allowed crate-wide on purpose: `module_name_repetitions`,
  `must_use_candidate`, `missing_errors_doc`, `missing_panics_doc`, `use_self`.
- **No `macro_rules!` for boilerplate.** Id newtypes, enums and impls are written out by hand; prefer explicit code.
- **Keep the core free of game concepts** (see the architecture rule above) and **ask before adding abstractions**. Keep
  types minimal.
- **Determinism is a contract.** No `HashMap` iteration order, wall-clock time, thread randomness or floating-point
  rules in the engine; randomness only through the `Rng` in the state; anything `apply` depends on must be in
  `GameState`. A new feature needs a test that replays.
- **Tests with the feature.** Engine tests live in `src/engine/tests.rs`; add one per new effect or action.
- `game-rules/` (rulebooks, card photos) is local only and gitignored; do not commit its contents.
- The maintainer does the committing and pushing; do not create branches or worktrees unless asked.

## Status

Working and tested: the core engine (zones, entities, pools and the bank, costs and payment enumeration, effects and the
resolution stack, events and triggers, decisions, reactions, modifiers, markets, flow, hidden information, replay), the
toy game, the CLI runner, and Res Arcana: card data, parser, compiler (all 73 base cards), `ResArcanaRules` (setup with
the mage draft and magic items, take/collect/actions/victory/cleanup phases, pass procedure, scoring and tiebreak),
a played-round test, random games that replay identically, and a heuristic bot that beats the random bot.

Simplifications: magic items have no passed side, only the first-game side of each place of power exists, and the
expansions are not transcribed.

The GUI exists (see above): the Rust side and the JSON protocol are tested; the page's JavaScript is syntax-checked but
not yet exercised in a browser or in the Tauri shell.

Not built yet: fuzz targets that drive the engine; real benchmarks; a performance pass (`apply` clones the state,
`legal_actions` is recomputed per call); the other side of each place of power; the expansions. `plan.md` sections 6
and 10 are the live status.

## License

MIT LICENSE

Copyright (c) 2026 Neil <neilhube12@gmail.com>
