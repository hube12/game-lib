# gamelib plan and knowledge base

Read **Status** first, then **Workflow rules**, then the sections you need. Nothing here is committed to git on purpose
(the owner commits; see Workflow rules).

## 1. Goal

A headless, deterministic Rust rules engine for board games (`gamelib`, working name; "rebrand later"), able to model
Twilight Imperium, Res Arcana, Wingspan, Finspan and Furnace. A Tauri viewer comes much later. Design principle: few
generic primitives, entities in zones, events, triggered effects, modifiers, a phase/turn state machine. The first real
game is **Res Arcana** (base game), then its expansions (Lux et Tenebrae, Perlae Imperii).

The original plan lives in a Claude Doc ("Board Game Engine - Core Plan",
https://claude.ai/artifact/96BqGnDqY1axT8YE4tYn21). This file supersedes it for implementation detail.

## 2. Status

Done and verified at the last green build (115+ tests, clippy `-D warnings`, nightly `fmt --check`, docs):

- Core crate `gamelib` (repo root): ids, `Rng` (ChaCha8, serializable), `Pool`/`Bank`/`transfer`, `Counter`, `Zone`/`Owner`
  (teams: `Owner::Players(Vec<PlayerId>)`), `CardDef`/`Entity`, `GameState` (zones, entities, draw with reshuffle),
  `Flow` (phases, `TurnOrder`, `RoundEndPolicy`, `GameEndPolicy`), `Market`, `Event`, `Modifier`, effects/abilities,
  resolution stack, `PendingDecision` (several players can decide at once), `Engine` (apply/legal_actions/view/replay),
  `PlayerView` with hidden information.
- `games/toy` (`gamelib-toy`): toy deckbuilder. `cli` (`gamelib-cli`): random/scripted bots, replay verification.
- `games/res-arcana` (`gamelib-res-arcana`): the **73 base-game cards as data** (`data/cards.ra`, rulebook symbols) and a
  parser into typed `CardSpec`s. 42 tests check the data against rulebook statements. Not connected to the engine yet.
- `afl-fuzz/` and `fuzz/` fixed and standalone (each has its own `[workspace]` table; otherwise IDEs fail with "current
  package believes it's in a workspace when it's not").

Done since: the **engine extension for Res Arcana** (sections 5-6, all steps), the compile step (`games/res-arcana/src/
compile.rs`, all 73 base cards), `ResArcanaRules` (`src/rules.rs`: setup, phases, pass, scoring), a played-round test,
random games with replay (`tests/round.rs`), `--game res-arcana` and a heuristic bot in `cli`. Res Arcana base game is
**playable**. Latest counts: 161 core tests, 39 compile, 6 round, 42 card-data, cli 11.

Not started: expansions; fuzz targets that exercise the engine; the engine performance pass.

## 3. Workflow rules (owner preferences and environment)

- **Never `git commit` or `git push`.** Only `git add <new files>`. Never create branches or worktrees unless asked.
  (A tool guard may ask for `EnterWorktree`; that conflicts with this rule, so edits in this repo have been made from the
  shell in the shared checkout. Ask the owner if that needs to change.)
- **No `macro_rules!` for boilerplate** (the owner asked for hand-written id newtypes). Prefer explicit code.
- Keep types minimal and **ask before adding abstractions** (the original plan says so). Everything added beyond the
  original plan is listed in section 4 so the owner can review it.
- Rust is not on the default `PATH` of the agent shell: `export PATH=$HOME/.cargo/bin:$PATH`. Toolchain is pinned in
  `rust-toolchain.toml` (1.99.1, it exists). `cargo +nightly fmt` is required (rustfmt options are nightly).
- Gates before saying "done": `cargo +nightly fmt --all -- --check`; `cargo clippy --workspace --all-targets
  --all-features -- -D warnings`; `cargo test --workspace --all-features`; `RUSTDOCFLAGS="-D warnings" cargo doc
  --workspace --no-deps --all-features`; and `cargo metadata --format-version 1 --all-features` in `.`, `fuzz` and
  `afl-fuzz` (the owner's IDE runs it on Windows at `C:\Users\r1\Documents\CODE\gamelib`, a different copy).
- Lints: crates use `#![deny(missing_docs)]` plus pedantic/nursery/cargo clippy. Allowed crate-wide on purpose:
  `module_name_repetitions`, `must_use_candidate`, `missing_errors_doc`, `missing_panics_doc`, `use_self`. `clippy.toml`
  allows the duplicate crate `syn` (transitive). Clippy also flags `assert!(x.is_empty())` without a message (use
  `assert_eq!(x.len(), 0)` or add a message), missing `const fn`, missing `Eq` derives, collapsible `if`s.
- `game-rules/` (rulebooks, card photos) is **gitignored on purpose** and local only.
- Scratch files go in the session scratchpad, not `/tmp`; delete temp dirs after use.
- The sandbox has `gs` (Ghostscript) and ImageMagick (`convert`, `identify`); no `pdftotext`, no Python imaging/OCR
  libraries, no `numpy`. PDFs: `gs -q -dNOPAUSE -dBATCH -sDEVICE=txtwrite -sOutputFile=out.txt file.pdf` gives text with
  heavy padding and doubled glyphs (collapse whitespace with python); icons are lost, so render pages with
  `-sDEVICE=png16m -r110` and read the image.
- AFL: `cargo install afl` fails (afl is a library); install `cargo-afl`, and after every toolchain change run
  `cargo afl config --build --force`. `-Zsanitizer=address` needs `cargo +nightly`; LeakSanitizer may need
  `ASAN_OPTIONS=detect_leaks=0` in sandboxes. See `afl-fuzz/run.sh`.

## 4. Engine design (what exists, and why)

Everything that happens goes through one loop in `src/engine.rs`:

1. An action pushes `Task`s on `state.stack`. Every state change queues an `Event` (`state.emit`).
2. `settle()` drains queued events first (modifiers expire, limits reset, abilities trigger, win check), then pops one
   task. Triggered effects are pushed on top, so they resolve before whatever was already waiting.
3. The loop stops when the stack is empty or `state.pending` (a `PendingDecision`) needs input. `Decide(i)` resumes it.
4. `apply()` validates the action by membership in `legal_actions` and works on a backup: on any error the whole state,
   events and log roll back. A rules loop hits `MAX_STEPS` (`RuleError::Runaway`).

Key decisions and conventions:

- Determinism: the seeded `Rng` lives inside the state; seed + action log = full replay (`Engine::replay`). Everything is
  `serde`-serializable and `PartialEq`; tests compare whole states.
- `GameRules` (per game): `setup`, `legal_actions` (default `standard_legal_actions`), `turn_end_effects`. Planned hooks:
  `phase_effects`, `pass_effects`, `winners` (section 5.4).
- Zones are generic: an `Owner`, `Order`, `Visibility`, `abilities_active`, optional `recycle_from` (reshuffle a discard
  into a deck when drawing). Per owner there is at most one zone per `ZoneKind`; effects refer to zones by kind.
- `Action::Buy{entity,pay}` / `Activate{entity,ability,pay}` carry `pay` = index into the enumerated payments, so the list
  must be deterministic for a state. `Decide(i)` picks option `i` of the pending decision.
- Abilities: `Trigger::{Activated, OnEvent(kind), React(kind)}`, `Condition`, optional `Cost`, `Effect`, `Limit`. Costs
  only apply to activated/react abilities. `Condition::InPhase(PhaseId)` lets a "collect" ability hook one phase (plain
  `OnEvent(PhaseStarted)` would fire for every phase).
- Flow: `Phase{id, interactive}`; non-interactive phases resolve automatically. `Flow::active_player()` is `None` outside
  interactive phases (found by tests: `turn_order.current()` returned the first player before the game started).
  `RoundEndPolicy::{FirstPass, AllPassed}`, `TurnOrderPolicy::{Clockwise, PassOrder}`, `actions_per_turn: Option<u32>`.
- Decisions: `Effect::Choose(options)` makes a pending decision for the controller. A multi-player pending decision needs
  the same options for everybody (options are effects resolved with the answering player as controller), so use
  controller-relative effects (for example "move the n-th card of my candidate zone").
- Hidden info: `PlayerView` hides decks and other players' hands; options of a pending decision are only shown to a
  player who still has to answer.
- Known limitation: `legal_actions` is recomputed per call and `apply` clones the whole state for rollback. Fine for the
  toy game and small tables; optimize before large games (Twilight Imperium).

Departures from the original Core Plan that the owner should review (all deliberate):

- `Engine::apply(player, action)` takes the player explicitly.
- `GameRules` has extra hooks and `legal_actions` has a default implementation.
- `Condition::InPhase`, `Cost::All`, `Effect::MoveAll`, `Pick`/`WithCard`, `ForEach*`, reactions (section 5) were added;
  `Custom(fn)` variants are omitted (function pointers are not serializable).
- `Cost::Discard(n)` from the plan is `Cost::Discard{from, n}` (the choice is enumerated like other costs).
- Teams: `Owner`/`OwnerRef::Players(Vec<PlayerId>)`; for triggers on a team zone the controller is the first member.

## 5. Engine extension for Res Arcana: design (decided, partly built)

Why: Res Arcana needs mechanics the first engine lacked. This was designed against the rulebook and the transcribed card
file, and is meant to stay generic (expansions and other games reuse it).

### 5.1 Payments (built: `src/cost.rs`, `src/pay.rs`, with tests)

- `Cost` variants: `Resources`, `Or`, `All`, `Exhaust` (= "turn"), `Any{n: Count, exclude, identical}` (any mix from the
  pool, optionally all identical; `Count::Var` = player-chosen N bound as `x`), `TakeFromCard{res,n}` (Athanor: take
  Elan off the card), `TurnOther{tag}`, `Destroy{which: This|Other|Own, tag}`, `Discard{from, n}`.
- `pay::enumerate(state, player, source, cost)` returns every concrete `Payment` the player can afford, tracking what is
  left of the pool as parts are paid, deduplicated and deterministically ordered (`BTreeSet`), capped at 4000. `Payment`
  carries resources, `exhaust`, `from_card`, `turned`, `destroyed`, `discarded`, `x`, and `single_paid()`.
- `pay::enumerate_discounted(state, player, cost, discounts)` is for placing artifacts: discounts are buckets
  `(units, kinds that cannot be reduced)`; each bucket removes exactly `min(units, reducible)` units in every possible
  distribution over the cost's kinds and "any" units (the player chooses what to keep paying); costs never drop below 0;
  buckets stack (rulebook: discounts are cumulative); a huge `n` means free. Simplification: an "any" unit can be reduced
  by a discount that excludes Gold.
- `CardDef.discounts: Vec<Discount{tag, n, exclude}>` are passive discounts a card grants in play (Artificer, Dragon
  Bridle, Dragon's Lair). Discounts apply to placing artifacts only, not to claiming monuments/places.

### 5.2 New effects (types built in `src/effect.rs`; resolution still to build in `engine.rs`)

`GainX` (N bound while paying), `GainChoose{n, exclude, identical}` (player picks a mix: enumerate multisets into `Gain`
options), `GainPaid` (Mermaid/Windup Man put the paid essence on a card), `GainAll{who}` (rivals gain), `GainPerRival
{res, counted}` (Treant / Hypnotic Basin: pick a rival, gain as many as they hold), `Pay`, `Exhaust`, `Ready`,
`ReadyAll`, `TakeAll{from}`, `Grow{per_kind}` (Windup Man), `Destroy` (resources back to the bank, card to its owner's
discard), `MoveNth`, `ResetCounters`, `ForEachPlayer`, `ForEachOwned{filter}`, `Pick{from, owner, filter, then}` +
`WithCard` (choose a card, then apply an effect with `Target::Card` bound; one candidate = no decision, none = nothing),
`Affect`/`AffectOne`/`Lose`/`Use`/`Ignore`, `ReactWindow`, `CheckWin`, `Reorder`/`Permute` (look at the top n
cards, pick an order), `PlaceCard{tag, discount}`, `DiscardChoose{n}`. `Ctx` carries `Bound{card, x, paid}`; a
`PendingDecision` stores the `Bound` too. `Effect::Choose` should drop options that are impossible right now.

### 5.3 Reactions, "affect" effects, win checks (built, generic)

Design rule: **the core crate names no game concept** (no Life, Dragon, victory check, essence). The game crate gives
meaning through data: `ReactKind(u16)` ids (like `ResourceId`/`ZoneKind`), tags, and effect composition. If a mechanic
needs a new engine feature, make it a generic primitive, never a game-named one.

- `Trigger::React(ReactKind)` abilities are not event-driven. They are offered by effects that open a window.
  `Effect::Use{source, ability, payment}` is the option that uses one (pays, then resolves the ability with its card
  as source and the payment's values bound; no action is counted). `activate` and reactions share `use_ability`.
- `Effect::Affect{who, defense, effect}`: aimed at every affected player in turn order after the controller (`Everyone`
  puts the controller last). Per player (`AffectOne`): skipped if `defense.skip_passed` and they passed; otherwise a
  decision with `[effect, one Use option per (react ability of a kind in defense.react, affordable payment), the
  defense.escape option]`; no alternatives = no decision. Choosing a reaction replaces `effect`.
  `Escape::{Pay, DiscardCard, Destroy(tag)}` is a way offered by the effect itself, not by a card.
- `Effect::Lose{res, n, shortfall}`: the controller loses `n` of `res`; for each unit missing they lose `shortfall`
  other resources of their choice (multiset choice), as far as they have them.
- `Effect::ReactWindow{kind, default, repeat}`: offers the controller their React abilities of one kind; with `repeat`
  the window reopens after each use. `Effect::CheckWin` asks `GameRules::winners` (default: the flow's
  `GameEndPolicy`, any `when`).
- A game builds its own compositions, e.g. a victory check = `Sequence[ForEachPlayer(ReactWindow{CHECK, Ignore, true}),
  CheckWin, ResetCounters{temp}]`; a Life loss = `Affect{Rivals, Defense{react:[LIFE_LOSS, (DRAGON_LIFE_LOSS)],
  escape, skip_passed:true}, Lose{LIFE, n, 2}}`. The Res Arcana constants live in `games/res-arcana`.

### 5.4 Hooks and flow (built)

- `GameRules::phase_effects(state, phase)`: effects resolved when a phase starts (controller = first player in turn
  order); abilities triggered by the phase's start event resolve first.
- `GameRules::pass_effects(state, player)`: resolved on `Action::Pass` before the turn ends; the pass is already
  recorded, so `turn_order.passed().first()` tells who passed first.
- `GameRules::winners(state)`: used by `Effect::CheckWin` (default: the flow's `GameEndPolicy`).
- `TurnOrderPolicy::FirstPasserStarts`: the first to pass starts the next round, the others keep clockwise order.
- `Engine::new` puts `Task::NextPhase` at the bottom of the stack, so tasks pushed by `setup` (drafting mages, picking
  items) run before the first round.
- `GameState.card_rules: Option<CardRules{hand, tableau, discard, on_discard}>` names the zone roles for `Action::Place`
  and `Action::Discard` (built, step 3).
- `MarketPolicy.source_buyable`: the top of the market's source can be bought blind (monuments in Res Arcana);
  `Market::offered`/`holds` are used by legal actions and `buy`.
- `Effect::Choose` drops options that cannot resolve right now (unaffordable `Pay`, or a `Sequence` whose `Pay` steps
  exceed the pool); no option left = no decision.
- Phases for a Res Arcana round (game crate): Take essences (non-interactive), Collect (non-interactive; abilities with
  `InPhase(COLLECT)`), Actions (interactive, `AllPassed`, 1 action per turn), Victory check (non-interactive), Cleanup.

### 5.5 Known simplifications to revisit

- Prism's second power: the rulebook text said "a different essence", but a later clarification says any N identical essences convert to N identical essences of any kind, so converting to the same kind is allowed (not a bug).
- Divination is modeled as "draw 3, then discard any 3 cards from hand" (rulebook wording: replace/discard that many).
- "Peek" (Hawk, Seer, Oracle): choose a deck (own artifact deck or the monument deck), look at the top 3, reorder, put
  back; FAQ: not on other players' decks; with fewer than 3 cards use what is there.
- Several `exclude` markers on any-essence symbols are read as "no Gold" (see 7.4).

## 6. Exact state of the working tree for the engine extension

Update this table whenever the engine is touched.

| File | State |
| --- | --- |
| `src/cost.rs` | rewritten: `Count`, `Which`, extended `Cost`, `Payment` (no state-free `Cost::payments()` anymore) |
| `src/pay.rs` | new, with tests: `enumerate`, `enumerate_discounted`, `own_cards`, `has_tag`, `combinations` |
| `src/effect.rs` | rewritten: `Target::Card`, `Bound`, `Ctx::new/with_card/for_player`, `Filter`, `Who`, `Escape`, all new effects, `ReactKind`, `Trigger::React`, state-aware `Condition::holds(state, controller, source)` with `SourceHas`, `SourceHasResources`, `All` |
| `src/card.rs` | `Discount`, `CardDef.discounts` |
| `src/state.rs` | `CardRules`, `card_rules`, `take_all`, `return_to_bank`, `take_from_card`, `grow` |
| `src/engine.rs` | see the status line at the bottom of this file |
| `src/engine/tests.rs` | updated for `discounts` and `Bound`; needs new tests for each new feature |
| `games/toy`, `cli`, `games/res-arcana` | untouched except `discounts: vec![]` in the toy `CardDef` |

Integration order that keeps the tree compiling at each step: (1) make `engine.rs` compile (use `pay::enumerate`,
`Ctx::new`, the new `holds`, `bound` in decisions, an explicit "unsupported" error for effects not built yet) and re-run
all existing tests; (2) simple effects (`Pay`, `Exhaust`, `Ready`, `ReadyAll`, `TakeAll`, `Grow`, `Destroy`, `Gain*`,
`MoveNth`, `ResetCounters`, `ForEachPlayer`, `ForEachOwned`, `Pick`/`WithCard`) with a test each; (3) `Place`/`Discard`
actions and `PlaceCard`; (4) reactions, `Affect`/`Lose`, win checks (generic, see 5.3); (5) hooks and flow changes; (6) `Reorder`/`DiscardChoose`.

## 7. Res Arcana knowledge

### 7.1 Sources and what is where

- `game-rules/res-arcana/[RA]_Rulebook_EN_FC_LR.pdf`: base rulebook (12 pages). `all_cards.jpg` (3958x2677): every card
  including expansion cards, German edition, best resolution. `components.jpg` (2911x2572): base components; the only
  place that shows the **Places of Power** and the **magic items**, and the 4 reference cards.
  `PerlaeImperiiRulesEN*.pdf` and `res arcana lux et tenebrae rules.pdf`: the two expansions (read, see 7.5).
- Cards are the **German edition**: ids in `data/cards.ra` are the printed German names; `en="..."` is the English name: 45 came from the rulebooks, the other 24 were added later (the user asked for English names
  everywhere) from knowledge of the English edition and are NOT read from an English card; the six least sure carry an
  `# EN-CHECK:` comment in the card file (Scholar, Solomon's Treasury, Jeweled Statuette, Elemental Spring, Cornucopia, Flame Pit).
  Four cards (Alchemist, Athanor, Mausoleum, Obelisk) have the same name in both languages and no `en=`.
- `all_cards.jpg` layout: row 1 has 15 cards, rows 2-8 have 16 (about 127 cards: 60 base mages/monuments/artifacts plus
  about 65 expansion cards). The crops used `x0 = 40*2.4737`, pitch `95.4*2.4737`, row top `(22 + 128.5*(row-1))*2.4737`
  (2.4737 = 3958/1600). First-column cards can be cut off; use `components.jpg` for those cost banners.
- Reading tips: crop 4 cards per image (about 1500 px wide) for readable icons; zoom strips to 650 px for tiny markers;
  verify surprising readings against `components.jpg` and the rulebook's printed examples (p3, p6, p7, p10 show cards).

### 7.2 Symbols (rulebook correspondence, also in the header of `data/cards.ra`)

Essences: `E` Elan (red), `L` Life (green), `C` Calm (blue), `D` Death (black), `G` Gold (yellow). `*` any essence (grey
pentagon, "3*" = any mix of 3), small icons next to a pentagon = excluded kinds (`*-G`, `*-GD`), `N` = chosen number,
`=` = identical essences. `T` = turning icon (turn this component). The triangle between cost and effect = `>`. `@` =
"on the component" icon. A red cross marks something paid. An orange tag = React (the condition is shown on the tag).
Card anatomy: placement cost in the top-left banner, collect ability in an oval over the art (black collect icon), powers
(cost > effect) below, victory points in a red circle at the bottom (places: "1+" = 1 plus per-essence points).

### 7.3 Rules that shape the engine (base rulebook)

- 2-4 players; the game ends when someone has 10+ VP at a victory check (13 with Perlae Imperii). Ties: most essences in
  pool, Gold counts double; still tied = shared win. A round: (1) collect: do Collect abilities and may take essences
  off components (all or nothing per component); (2) actions, one per turn clockwise from the first player until all
  passed: place an artifact, claim a monument or place of power (monuments always cost 4 Gold; claim one of the 2
  displayed or the top of the deck; displayed ones are replaced), discard a card for 1 Gold or any 2 other essences, use
  a power on a straightened component, or pass; (3) victory check, then straighten everything.
- Pass: the first to pass takes the First Player token (1 VP when checking victory) and flips it; swap your magic item for
  a different one (flipped to its Passed side); draw 1 card. A passed player cannot act and ignores Life loss but still
  receives essences from rivals' powers. Reshuffle the discard when drawing from an empty deck.
- A power = cost > effect; using one is an action; a power that turns its component cannot be used again until it is
  straightened; powers without turning can be used repeatedly (each use is an action). React powers are used out of turn
  and take no action; a React on a turned component works unless its cost needs turning it.
- Essences must be paid from the pool unless a power says otherwise (Athanor takes Elan off the card). Putting an essence
  "on the component" takes it from the supply; essences on components are collected later; a destroyed card's essences
  go back to the supply. The supply is unlimited (use 5x chips).
- Life loss: each affected rival loses N Life; for each Life missing they lose any 2 other essences (Gold included) if
  possible. Passed rivals ignore it. Dragons give rivals an immediate React printed on the card ("may discard a card /
  pay X / destroy an artifact to ignore").
- Discounts are cumulative with each other and with the power being used to place a card (Crypt, Dragon Egg, Dragon
  Tooth); costs never go below 0; some discounts exclude Gold. Two artifacts (Magical Shard, Prism) cost 0 (placing them
  is still an action).
- FAQ (expansion rulebooks, apply to the base game too): discarding as part of a power's cost does NOT give the discard
  action's Gold/essences; turning a different artifact as a cost does not trigger its powers; you can put an essence on a
  turned component with a power (Mermaid on Cursed Forge); draw-and-put-back powers cannot target other players' decks; a
  power with no effect may be used if its cost is fully paid; victory-check reactions (Golden Statue) work for mid-round
  and end-of-round checks.
- Setup (non-first game): shuffle the 5 places of power (random side); 2 monuments face up + the rest as a deck; 1 of each
  essence per player; deal 2 mages and 8 artifacts per player (keep 1 mage; shuffle artifacts; draw 3); items are picked in
  reverse order starting right of the first player. Draft variants and a 3-game match variant exist (rulebook p9).

### 7.4 Card data: conventions and the readings worth re-checking

`games/res-arcana/data/cards.ra` documents its own grammar in its header: `power <cost> > <effect>`, `react <trigger>
<cost> > <effect>`, `collect`, `collect-left`, `collect-grow`, `collect-curse`, `onclaim`, `discount`. Counts: 10 mages,
10 monuments, 5 places of power (one side each), 8 magic items ("Tod / Leben" and "Ruhe / Elan" are one card each), 40
artifacts (6 dragons, 7 creatures, 12 worth 1 VP). 42 tests in `games/res-arcana/tests/cards.rs`.

Hard-to-read items (flagged `# CHECK:` in the file or noted here); verify against real cards when possible:

- Tiny markers beside a pentagon are read as "excluding that essence" (Gold; Himmlisches Pferd shows Gold+Death, which
  matches the rulebook's "no Gold or Death" example). Dragon discounts (-3, -4) show no marker; Artificer (-1) and Crypt
  (-2) do.
- Goldene Statue's power includes a turn icon (turn + 3 Gold for +3 temporary VP).
- Cursed Forge power: pay 2 Elan + 1 Gold, put 1 Gold on it. Divination: draw 3, discard 3. Prism's second power and
  Philosopher's Stone: "N identical essences" conversions (Stone: pay 2 any + N identical, take N Gold).
- Sea Serpent lets rivals destroy one of their own artifacts to ignore; Wind Dragon: discard a card; Fire/Water/Earth/Bone
  Dragon: rivals pay 1 of Calm/Elan/Gold/Death.
- Corrupt Altar's first power costs 2 Life (not 1) and its second power may destroy itself; Fiery Whip's may not.
- Guard Dog's first power (1 Elan: straighten self) works while turned (rulebook p10).
- Places of Power are double-sided; the back sides are not photographed. The rulebook names Sacrificial Pit and
  Sorcerer's Bestiary (probably backs). VP: Cursed Forge 1 + 1/Gold, Sacred Grove 2 + 1/Life, Dragon's Lair 1/Gold,
  Coral Castle 3, Catacombs 1/Death.

### 7.5 Expansions (rules read, cards not transcribed)

- **Lux et Tenebrae**: 5 players; monuments / places of power by player count (2p: 7/4, 3p: 10/5, 4p: 12/6, 5p: 14/7); new
  artifact type **Demon**; **scrolls** (one-use powers claimed with the Inscription magic item, saved until used, then
  returned to the center); "retrieve" (take a card from your discard into hand); more mages/items/monuments/places/
  artifacts; Illusion item (React as you use a power to satisfy turning a Creature/Demon/Dragon); Catherine Wheel
  inflicts Life loss on **all players**, owner included; Projection (3:1 Calm to Gold).
- **Perlae Imperii**: new essence **Pearls** (a 6th kind; excluded by "non-Gold" style restrictions; 1 VP each at a
  victory check wherever they are, not counted for ties; convertible at any time into 1 Gold or any 2 non-Gold non-Pearl
  essences, also out of turn when reacting); target becomes **13 VP**; 5 players; each player starts with a Pearl; a
  Reminder tile; Raven Familiar (gain a Gold when taking one), Phoenix, and more.
- Engine consequences: essence kinds stay data-driven (`ResourceId` + `Bank`), card types are tags
  (Dragon/Creature/Demon), reactions and "retrieve" are effects, scrolls are one-use abilities (`Limit` + moving the card),
  pearls need a conversion that can happen during other players' decisions.
- The expansion cards are in `all_cards.jpg` but the base rulebook does not define their symbols; use the two expansion
  rulebooks and expect new icons (pearl, demon, scroll, retrieve).

## 8. Remaining work (ordered)

### 8.1 Engine (finish what is in progress)

1. Make `src/engine.rs` compile (section 6 order) and keep the 115+ existing tests green.
2. Implement and test every new effect and action in 5.2-5.4. Add tests in `src/engine/tests.rs` per feature:
   pay/take/grow/destroy, choose-mix gains, pick/with-card, place and discard actions (with discounts), reactions (ignore by
   reaction, passed players immune, escape options, losing 2 other essences), victory check with temporary VP and
   tiebreak, hooks (`phase_effects`, `pass_effects`, `winners`), `FirstPasserStarts`, setup tasks before `NextPhase`,
   claiming the top of the monument deck.
3. `PlayerView`: show components with their essences, keep hiding other players' pending options (already done).
4. Performance pass: avoid cloning the whole state in `apply`, cache legal actions per state revision.

### 8.2 Res Arcana game crate (`games/res-arcana`) - items 1-4 DONE except per-card power tests (in progress)

1. A compile step from `CardSpec` to `CardDef` (tags: kind, dragon, creature; cost; `on_gain` for `onclaim`; abilities
   with `InPhase(COLLECT)` conditions; discounts; react abilities).
2. `ResArcanaRules: GameRules` with setup (zones, bank, shuffling, mage draft as a simultaneous pending decision, items in
   reverse order), phases (take, collect, actions, victory check, cleanup), pass procedure, first player token, scoring
   (card VP + per-essence VP + token + temporary), tiebreak, hidden mage/hand information.
3. Tests: every card's powers once in a controlled situation, a full random game to the end via `gamelib-cli`
   (`--game res-arcana`), replay determinism, 2-4 players.
4. Bots: random play is not enough to judge the game; add a simple heuristic bot (claim monuments, use collect engines).

### 8.3 Data

- Re-check the `# CHECK:` readings against real cards; transcribe the other side of each Place of Power.
- Transcribe the ~65 expansion cards once their symbols (pearl, demon, scroll, retrieve) are defined; keep the same file
  grammar and add the new keywords to the parser, with tests that quote the expansion rulebooks.
- Optional: "first game" preset hands/mages (rulebook p4) as data.

### 8.4 Quality and tooling

- Fuzz targets that feed bytes as action choices into the toy and Res Arcana engines (no panics, replay equality). The
  owner said: not for now.
- Dockerfile, `Cross.toml`, `just docker`, `cargo deny/audit/tarpaulin` could not be run in the sandbox (tools missing or
  not testable); run them on the owner's machine.
- README: describe the engine concepts and the card file grammar for new contributors.

## 9. Pitfalls and lessons learned

- Tests found real bugs: `current_player()` before the game started; triggers firing on every phase (hence `InPhase`).
  rustfmt-aligned struct fields break string-based code edits (run `cargo +nightly fmt` first, or match the exact text).
- Clippy 1.99 with `-D warnings` is strict: collapsible ifs (use let-chains, edition 2024), `needless_pass_by_value`,
  `option_if_let_else`, `semicolon_if_nothing_returned`, `iter_cloned_collect`, `assert_is_empty`.
- A package under a workspace root that is not a member needs its own `[workspace]` table; `exclude` alone did not stop
  the cargo error for `afl-fuzz`.
- When a parser splits on `:` and `+`, tokens that contain those characters (`~pay:1D`, `+2`) must be peeled off first;
  bonus numbers are written without `+` in the card file for this reason.
- Photo transcription: always zoom before trusting an icon (the Sea Serpent, Corrupt Altar and Guard Dog were wrong on
  the first read); cross-check with the rulebook's printed examples and fine-points text.
- `Payment` index stability: `legal_actions` and `apply` must enumerate with the same state; never reorder enumerations
  between the two.
- The photographed Places of Power sides are the "first game" sides; do not assume they are the only sides in the box.

## 10. Live status line (update last)

Engine integration step 1 is DONE: `src/engine.rs` compiles against the new `cost.rs`/`effect.rs`/`pay.rs`, all tests pass
(108 core + toy 10 + cli 8 + res-arcana 42), the toy game and CLI still work. Implemented in `apply_effect`: `Gain*`
(`GainX`, `GainPaid`, `GainAll`), `Pay`, `Exhaust`, `Ready`, `ReadyAll`, `TakeAll`, `Grow`, `Destroy`, `MoveNth`,
`ResetCounters`, `ForEachPlayer`, `ForEachOwned`, `Pick`/`WithCard`, `Ignore`; payments (take from card, turn other,
destroy, discard) are applied in `Engine::pay`. These new effects have no engine-level tests yet (only `pay.rs` is tested).
Not built: the `GameRules` hooks (`phase_effects`, `pass_effects`), `FirstPasserStarts`, `Choose` option filtering, `NextPhase` at the stack bottom, market top-of-deck.

Step 2 (tests for the already-implemented effects) is DONE: 13 engine tests at the end of `src/engine/tests.rs` (`Pay`,
`Exhaust`/`Ready`/`ReadyAll`, `TakeAll`, `Grow`, `Destroy`, `GainX`, `GainPaid`, `GainAll`, `MoveNth`, `ResetCounters`,
`ForEachPlayer`, `ForEachOwned`, `Pick`, and cost-bound `Target::Card` for destroy/discard costs). They found a real bug:
`Engine::activate` never filled `Ctx.bound` from the payment, so `GainX`, `GainPaid` and `Target::Card` had nothing bound.
Fixed (bound = `payment.x`, `payment.single_paid()`, destroyed-or-first-discarded card). Latest green gates: fmt, clippy
`-D warnings`, 121 core + toy 10 + cli 8 + res-arcana 42 tests, docs, `cargo metadata` in `.`, `fuzz`, `afl-fuzz`. The
`assert_is_empty` clippy failure in `pay.rs` is fixed. The test rules for the extension use `ra_table`, `find`, `use_ability`/`use_def` helpers (same file).

Step 3 is DONE: `Action::Place{entity, pay}` (pay = index into `pay::place_payments`: the card's cost minus the discounts
of the owner's cards in play, `pay::discount_buckets`), `Action::Discard{entity}` (moves the card to the discard, then
resolves `CardRules::on_discard` with no source card) and `Effect::PlaceCard{tag, discount}`. Both actions are offered by
`standard_legal_actions` only when `state.card_rules` is set, for cards in the player's own hand, and count as the turn's
action. `PlaceCard` builds a `Choose` whose options are `Sequence[Pay.., WithCard{Move to tableau}]` for every
(hand card with the tag, payment) pair, plus a final `Ignore` option (placing is optional); no affordable card = no
decision. No new events were added (a placement is an `EntityMoved`); cost modifiers (`CostDelta`) are not applied to
placement. 6 new tests.

Step 4 is DONE and generic (see 5.3): `Affect`/`AffectOne`/`Lose`/`Use`, `ReactWindow`, `CheckWin`, `GameRules::winners`,
`ReactKind(u16)` in `id.rs`, plus `GainChoose`, `GainPerRival`, `DiscardChoose`, `Reorder`/`Permute`
(`State::reorder_top`). The earlier game-named variants (`LoseLife`, `SufferLifeLoss`, `VictoryCheck`,
`ReactKind::{LifeLoss, DragonLifeLoss, VictoryCheck}`) were removed on purpose. 149 core tests. Every `RuleError::Unsupported`
effect is gone. Remaining engine work: section 5.4 (hooks and flow) and 8.1 items 3-4.

Step 5 is DONE (see 5.4): hooks, `FirstPasserStarts`, setup-before-first-phase, market top-of-source, `Choose` filtering,
with 9 more tests (158 core tests). Open engine item: the performance pass (`apply` clones the whole state for rollback,
`legal_actions` is recomputed per call); not needed for Res Arcana, do it with a benchmark before Twilight Imperium.

Res Arcana game crate DONE (see 8.2). Things worth knowing:
- Engine additions made for the four cards the compiler first rejected: `Count::CostOf{discarded, bonus}` (size a gain
  by the placement cost of the card bound while paying), `Bound.discarded` (the first discarded card, separate from the
  destroyed one in `Bound.card`), `Effect::PlaceCard.from` (`None` = hand, e.g. `Some(DISCARD)` for the Crypt).
- `rules.rs` zone kinds: places 11, monument row 12, items 13, candidates (dealt mages) 14, unused mages 15, aside 16;
  counters VP 0, TEMP_VP 1, FIRST_TOKEN 2. Setup: mage choice is a simultaneous pending decision, then magic items are
  picked in reverse turn order (stack tasks), then `NextPhase`. `EndTurn` is removed from the legal actions (a turn is one action).
  Pass: first passer takes the token (+1 VP at a check), everyone swaps their item (old one set aside while picking),
  draws 1. Winner: score >= 10 at a victory check, tiebreak essences with Gold counted twice, equal = shared win.
- Random 2-4 player games always finish (about 10-19 rounds); wins are spread evenly over the seats. The heuristic bot
  (claim monuments/places, discard for Gold while under 4, place valuable artifacts, powers last, 7 actions per round)
  beat the random bot in >= 55% of 160 two-player games (a measured 59% on 80); a larger action budget per round was
  better than a small one, so passing early does not pay.
- Verify against the rulebook later: the monument/place VP and costs come straight from `cards.ra`; item passed sides are not modeled.

GUI (new): `gui/` = `gamelib-gui` (Session, Snapshot/BoardView DTOs, Backend + JSON protocol, Describer, shared no-build web
modules, optional `serve` feature with tiny_http), `games/res-arcana-gui/` (ResArcanaUi, page, `res-arcana-dev` server) and
`games/res-arcana-gui/src-tauri/` (standalone Tauri 2 shell, not a workspace member, not buildable in the sandbox: no
webkit2gtk). To share bots without the GUI depending on `cli`, `Bot`/`RandomBot`/`ScriptedBot` moved into the core
(`src/bot.rs`) and `HeuristicBot` into `games/res-arcana/src/bot.rs`. Found while building it: rivals could not see which
card a player placed, because every card was created face down; Res Arcana now creates cards face up (zone visibility
protects hidden information). Tested: 4+6 gui tests, 5 backend tests (a person plays 300 actions through the JSON API for
2/3/4 players; hidden names never leak; the backend is `Send`). NOT yet verified: the JavaScript at runtime in a browser
(only syntax-checked with oxc, `cargo build` of a scratch crate) and the Tauri shell. Downloading a browser to test was
blocked by the sandbox, so run `just gui-dev` and look.
Known card bugs left (ignored tests in `games/res-arcana/tests/powers.rs`): Divination with a short deck discards as many as
it should draw (needs "discard as many as drawn"). Chalice of Life and Fountain of Youth were fixed (`2C@+L@` in the card file).

Behavior change to remember: `pay` in `Action::Buy`/`Activate` is now an index into the payments the player can AFFORD
(`pay::enumerate`), not into every alternative of the cost.

No known failures. Beware: rustfmt removes the trailing `;` after `d.abilities = vec![...]` when it is the last statement
of a closure, and clippy (`semicolon_if_nothing_returned`) then complains; add the `;` back after formatting.

## 11. Session of 2026-11-15: planner bot, game logs, rule fixes, GUI additions

- **Bot**: `PlannerBot` (`games/res-arcana/src/planner.rs`) replaces the heuristic bot in the GUI; `--bot planner` and
  `--bots a,b` (per seat, seeds paired across seatings) in the CLI. How it decides, its measured strength, weaknesses
  and how to use the logs: `games/res-arcana/STRATEGY.md`. Read that before touching the bot.
- **Core additions (generic)**: `Engine::from_state`, `GameRules for &R`, `Bot::choose_with_state` and `Bot::explain`.
- **Game logs**: `gui/src/record.rs`; `Backend::with_log_dir`; `backend_with_logs()` writes
  `game-logs/res-arcana/*.jsonl` (gitignored, `GAMELIB_LOG_DIR` overrides). `res-arcana-dev selfplay` logs bot games.
- **Rule fixes** (in `ResArcanaRules::legal_actions`, `rules.rs`): powers of a turned component cannot be used, except a
  power that only straightens its own card (Guard Dog); a "place an artifact" power needs something placeable and then
  the placement cannot be declined. The second one is the owner's table rule: the FAQ of both expansion rulebooks says a
  power with no effect may be used if its cost is paid. Remove the `places` check in `power_allowed` to go back.
- **Collect step**: collect abilities are `React(COLLECT_STEP)` abilities, once per round; every card also gets a "take
  everything on this card" ability (added in `ResArcanaRules::new`, not printed). The TAKE phase opens one window per
  player (`collect_step()`); the player orders the steps, and cannot end it while a collect ability is owed. Vault and
  Windup Man still trigger in the COLLECT phase, after the window.
- **GUI**: a decision whose options are abilities of the viewer's cards is shown as buttons on those cards plus "Done"
  (`decidedOnCards`); looking at the top 3 cards shows them (`Session::looked_at`); payment menus are no longer covered
  by the hover popup; planning mode (`Session::plan_*`, requests `plan_start|undo|redo|run|stop`, `planPanel` in
  `app.js`).
- **Not verified**: the page JavaScript at runtime (syntax-checked with oxc only), as before.
