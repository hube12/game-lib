# Res Arcana bot: how it decides

The bot is `PlannerBot` in `games/res-arcana/src/planner.rs`. The GUI seats it at every bot seat; the CLI calls it
`--bot planner`. The older `HeuristicBot` (`src/bot.rs`) is kept only as a baseline to measure against.

State of this document: 2026-11-15. Numbers come from self-play only. No game against a person has been logged yet, so
nothing here is tuned against human play.

## 1. The idea in one paragraph

On every choice the bot copies the game, plays each legal move on the copy with the real engine, answers the decisions
that follow, and scores the position it ends in. It keeps the move with the best score, and passes when no move improves
the position. All card knowledge comes from the compiled card data, so there is no per-card script: a new card is
understood as soon as it compiles. What makes the bot good or bad is the scoring function, described in section 3.

## 2. The search

1. **Blind copy.** The bot gets the full state (it must run the engine), but first shuffles its own deck and the
   monument deck in the copy and never reads a rival's hand or deck. A simulated draw is therefore a random card it
   could draw, not the real next card.
2. **One move ahead.** Every legal action is applied to the copy (`Engine::from_state` + `apply`).
3. **Decisions after the move.**
   - Its own follow-up decisions (which essences to gain, which card to straighten) are tried in full for the first two
     in a chain, then answered greedily.
   - A rival's decision (pay to ignore a Dragon, which essences to lose) is answered with what is best for that rival at
     first sight. So an attack is valued by what it really costs the rival, escape included.
   - The simulation stops at a decision several players take at once, and at the start of the next round.
4. **Choosing.** Outside a decision: best non-pass move if it gains more than `min_gain` (0.05), otherwise pass. A pass
   that wins the game is taken over anything that does not win. Inside a decision: the best option.
5. **Shortcuts.** One legal move: play it. Mage draft (simultaneous): keep the mage with the higher income. Collect step:
   plain "gain" collect abilities are done first without simulating; only a curse payment or taking essences off a card
   is thought about, once everything else is collected.
6. **Guards.** At most 4000 simulated moves per choice and 60 actions per round.

Cost: about 0.2 s per bot per whole game in a release build.

## 3. The scoring function

`position = my value - average of the rivals' values`, each in "essence units" (one ordinary essence is about one unit).
A win is +10000, a loss -10000.

Two numbers are fixed per choice, from the state before the move:

- **Rounds left** = `(10 - best score at the table) / 2.2`, between 0.4 and 4.
- **Worth of a victory point** = `5 + 6 / rounds left`. About 6.5 units early, 11 with one round left, 20 at the end.

A player's value is the sum of:

| Part | How it is counted |
| --- | --- |
| Victory points | score x worth of a point. The first-player token counts little early and as a full point near the end. |
| Essences in the pool | 0.8 each, Gold 1.6. |
| Life as a shield | +0.6 for each of the first 2 Life while a rival has a Life-loss power in play. |
| Essences on cards | 0.9 of their pool worth, unless the card scores them (then they are points, so the bot leaves them). |
| Components in play | income per round x rounds left, plus half of the best turning power's gain while the card is still ready. |
| Cards in hand | 1.2 each (a card can always be discarded for Gold), +1.0 for a Dragon. |
| Cards worth getting | see below. |
| Golden Statue | about 1 point, 2 with 3 Gold in the pool. |

**Income of a card** is read from its abilities: collect amounts; the best power that turns the card (gain minus cost,
essences put on a scoring card counted as points, Life loss counted per rival, essences given to rivals subtracted);
half of the profit of repeatable powers; a little for a Life-loss reaction, more when a rival can attack. A power that
needs another card type (Dragon's Lair turning a Dragon) only counts when the player owns one.

**Cards worth getting** (the planning part). For every artifact in hand, both face-up monuments and every place of
power:

- missing essences = cost minus pool, after the player's placement discounts;
- rounds needed = missing / flexible income (0.7 x essence income + 0.5 per hand card, at least 2);
- profit = points x point worth + income x (rounds left - rounds needed) + on-claim gain - price - (1.2 if from hand);
- counted share = profit x 0.85 x 0.7^(rounds needed).

The five best are added up, each reserving its essences so the same Gold is not promised twice. This term is what makes
the bot save 4 Gold for a monument, gather Death for the Catacombs, or keep a card instead of discarding it. Buying a card
turns the 85% promise into the full value, which is why affordable good cards get bought.

## 4. Dragons

- A Dragon's power is worth 2 Life x 0.75 per rival per round in the income estimate, so Dragons rank well as cards to
  get; the +1.0 hand bonus stops the bot from discarding them for Gold early.
- The attack itself is simulated: the rival loses 2 Life, or 4 other essences when it has none, or pays the escape. The
  bot sees the real damage and the damage to the rival's plans (the "cards worth getting" term of the rival drops).
- Moves that hurt rivals get +0.4 so they are made early in the round, before the rival passes and becomes immune.
- Dragon's Lair, Dragon Bridle, Dragon Egg and Dragon Tooth need no special code: discounts lower the missing essences
  of Dragons in hand, and turning a Dragon for the Lair is compared with attacking by simulation.

## 5. Measured strength (self-play, release build, seeds paired across seats)

| Match | Result |
| --- | --- |
| planner vs old heuristic bot, 400 games | 400 - 0, average 6.1 rounds |
| planner vs random bot, 300 games | 300 - 0, average 6.3 rounds |
| planner vs itself, 2 players, 300 games | average 5.84 rounds |
| planner vs itself, 3 players, 120 games | average 5.78 rounds |

The rulebook says a game typically lasts 4 to 6 rounds. The old bot needed 10 to 19 rounds against random play.

**Tuning.** Each parameter was changed alone and played 600 games against the defaults: `vp_late` 10 (301-301),
`attack` 1.6 (286-316), `dragon` 2.5 (277-325), `dragon` 0 (312-291), `pace` 2.8 (292-310), `hand_card` 0.8 (300-303),
`goal_decay` 0.55 (300-305), `rival` 0.6 (302-299), `ready` 0.3 (307-293), `essence` 1.0 (290-312). All within noise
(about +-25) except a larger Dragon bonus, which looks slightly worse. An earlier, biased run also showed `horizon` 3
clearly worse than 4. The defaults were kept. The scoring is flat around them, so more self-play tuning is unlikely to
help; human games are the next source of information.

A warning about method: the first sweep compared bots without playing each deal from both seats and reported 223-177
for two identical bots. The CLI now plays every seed once per seating (`seed_of` in `cli/src/main.rs`).

## 6. Known weaknesses

- **One move deep.** Multi-step lines are only found through the "cards worth getting" term. The bot does not plan
  "straighten, then use again" or hold Life back for a specific later purchase.
- **Rivals are modelled thinly.** It never predicts a rival's next action, does not race for a contested place of
  power, and does not pass early to deny the first-player token on purpose.
- **Draw and peek powers are undervalued.** A draw is a flat 1.3 units; looking at the top 3 cards is worth almost
  nothing to it, so it rarely uses Seer, Hawk or Oracle.
- **Rough estimates** for variable powers (Athanor, Philosopher's Stone, Prism), destroy powers and Windup Man.
- **Discards freely** in round 1 to rush a place of power. Strong in self-play; unknown against a person.
- **Pass timing** is "nothing gains more than 0.05", not a decision about tempo.

## 7. Game logs: how the bot will be improved from your games

Every game played in the GUI is written to `game-logs/res-arcana/<date>_<time>_seed<N>.jsonl` (set `GAMELIB_LOG_DIR` to
move it; the folder is gitignored). One JSON object per line:

- `start`: seed, seats, which are human.
- `round`: at every new round, each player's pool and every zone with its cards (hands and decks included, so the file
  is for analysis, not for showing during play), plus the table.
- `action`: who, human or bot, the raw engine action, the move in words, round and phase, scores and pools after it. Bot
  moves carry `bot`: rounds left, point worth, position, and the best six candidates with their gain.
- `end`: winners, round, final positions.
- `undo` (only with "Auto next player" off): `removed` moves were taken back, back to action number `to`; the `action`
  lines with `n` above `to` that came before it did not happen. Drop them before replaying.

Seed plus the raw actions replay the game exactly (`Engine::replay`), so any position can be rebuilt and the bot asked
what it would have done there.

What to look for in a human game:

1. Rounds to 10 points for the person against the bot, and where the score curves separate.
2. Human moves the bot scored low or negative (replay, evaluate the human's move): each is either a human mistake or a
   hole in the scoring function.
3. Bot moves with a tiny margin over the second choice in rounds 1 and 2: the opening is where one-move lookahead is
   weakest.
4. Dragon attacks: how often the rival escaped cheaply, and whether attacking first or last in the round did more.

Self-play games can be logged the same way:
`cargo run --release -p gamelib-res-arcana-gui --features serve --bin res-arcana-dev -- selfplay <games> <first seed> <players>`.

## 8. Commands

```sh
# bot against bot, seats rotated, every deal played from each seat
cargo run --release -p gamelib-cli -- --game res-arcana --bots planner,heuristic --games 200
# try other numbers against the defaults
cargo run --release -p gamelib-cli -- --game res-arcana --bots planner:attack=1.6:pace=2.5,planner --games 600
# play against it in a browser (logs are written automatically)
just gui-dev
```

Parameter names are the fields of `planner::Params`.

## 9. Findings from the first human game (2026-11-15, seed 1, human won 12-11 in round 5)

The bot ran a Death-only engine around Catacombs (about 1-2 VP a round: 1, 3, 6, 8, 11). The human scored 0 until round 2,
then 8 points in rounds 4-5 from a stockpile (Dragon's Lair Gold at 1 VP each, Coral Castle 3, Obelisk, Sacred Grove,
Library) and ended the game with Coral Castle's victory-check while ahead. The bot passed first at 11-11 holding
unused Death 3, Gold 2, Life 2, so the human made the last moves.

Improvements, most important first:

1. **Pace is underestimated.** `rounds left = (10 - best)/2.2` assumes about 2.2 VP a round. Derive the rival's pace from
   its pool, stored essences and the places and monuments it can afford, so the bot hurries earlier.
2. **Pass timing and last-mover advantage.** Passing first lets the rival finish unopposed. Evaluate the rival's best
   remaining actions this round before passing; at a tie or near-tie keep acting and spend idle resources.
3. **Coral Castle's victory-check is not modelled.** A rival owning it can end the game when ahead. Count it as a threat,
   and use it when the bot leads.
4. **Gold hoarding for monuments.** Monuments cost 4 Gold; the bot never had more than 2 and claimed none. Try a milder
   decay for monuments in the last 2 rounds, and a bonus for not discarding cards for Gold when a monument is in reach.
5. **Contested places of power.** The Lair and Coral Castle gave the human about 7 of 12 points. Value denying a place
   the rival is close to affording (also listed in section 6).
6. **Engine-first openings and combos.** The human used Reanimate to straighten Prism, Chalice and the Lair for a second
   use. A two-step "act, straighten, act again" evaluation, or a bonus for straighten effects on strong turning cards,
   would help.
7. **Idle resources.** Essences that cannot become points or a purchase before the game ends should be penalised so the
   bot trades them in earlier.

## 10. Findings from the 4-player draft game (2026-11-18, seed 12939221)

The three bots drafted and placed Dragons (Bone, Water, Fire, Earth) and never attacked with them. At every decision the
attack scored about -1.7 against +1.5 for passing.

Cause: a Dragon's attack was estimated as the sum over all rivals, but the position subtracts the *average* of the rivals,
so a ready Dragon was worth about 3x more unused (the `ready` term) than the attack was worth once played. Using it
always looked like a loss. The Water and Fire Dragons (escape: pay 1 Elan / 1 Calm) were hit hardest, because the rival's
best answer is a cheap escape.

Changes:

- `attack_ready` (default 0.25): share of an attack that counts in the ready term. In 40 three-player self-play games
  the bots attacked 57 times before, 70 with 0.5 and 80 with 0.25; Water and Fire Dragons now attack too (10 and 5
  times, 0 before). Strength is unchanged: 202-200 and 201-200 over 400 paired games.
- Draft pick: an artifact is now valued as `income x rounds left x draft_time + points x point worth x draft_vp +
  Dragon bonus - price x draft_cost` (0.5 each), instead of income alone. In 1300 paired draft games this is level with
  the old rule (about 50%), so it is a sanity fix, not a measured gain.

## 11. Findings from the Catacombs game (2026-11-22, seed 100000, human won 10-4 in round 4)

The human drafted Witch (collect Life/Death) and Death / Life, so it started the first action phase with 3 Death against
the bot's 1 (Duelist, Calm / Elan). Both then discarded one card per turn for 2 Death, the bot reaching 7 when the human
reached 9 and claimed Catacombs of the Dead in round 1. The Catacombs then made about 2.5 VP a round (10 Death on the
card = 10 VP). The bot sat on 7 Death it could not spend, passed with 20 idle essences in rounds 2-4 and scored 4.

The race was lost in the draft, not in round 1. What the bot lacked was any idea of it:

- A place of power is one card, but each player counted it as his own goal, so a rival 2 Death ahead did not lower the
  bot's estimate of its own chances (and the bot's lead did not show up as a loss for the rival).
- A price of 9 Death was reached "by any essence income", so a collect of Death and a collect of Elan looked the same,
  and the draft never preferred a Death mage for a table with the Catacombs.

Changes (all three are off at `race_lag=0 place_pull=0 other_kind=1`):

- `race_lag` (0.5), `race_floor` (0.2): for every place and monument the rounds each player needs to afford it are
  compared; the player who is behind counts its profit 0.5 less per round of lag (at least 20% stays). `position` now
  scores all players first and settles the races afterwards (`Eval::parts` and `Eval::finish`). Effect: a bot that is
  behind stops sinking its hand into Death (in the logged game it takes other essences at the discard choices) and a
  bot that is ahead gets the full value and the rival's loss, so it hurries to claim first.
- `other_kind` (0.35): a price with 6 or more of one essence (9 Death, 8 Life, 6 Elan) is reached by the income of that
  kind (fixed, "A / B" choice, or free choice), plus half a card per card in hand (discard for 2), plus this share of
  every other kind. A Witch or Death / Life now halves the rounds to the Catacombs.
- `place_pull` (3.0): in the mage choice, a mage's income gets this many units per essence it collects of a kind a
  place on the table is made of. In the logged game the bot's choice (Duelist) is unchanged, because Healer and Duelist
  cannot collect Death; with a Necromancer or Witch in the pair it will take it when the Catacombs are on the table.

Measured: 400 paired self-play games against the same bot with the three switched off, 197-204 (noise). The change is
about contested play, which self-play seldom produces, so it is not expected to move that number.

Not solved: the bot still cannot stop a player who is simply ahead in the draft. The remaining weakness is idle essences
after a lost race (section 9.7) and converting them into points faster than 2.5 VP a round.

## 12. Findings from the replayed Catacombs game (2026-11-22, seed 100000, human won 10-4 in round 4)

Same seed and same human line as section 11, played after the `race_lag` / `place_pull` / `other_kind` changes. The bot
drafted the same Duelist and Death / Life, so those changes did not alter the draft. Result: 10-4 again (8-1 at the
start of round 4 before, 8-2 now).

The game was decided in the draft. The bot acts first in round 1 but could reach at most about 8 of the 9 Death needed
for the Catacombs, even by discarding its whole hand. Dragon attacks were no use: the Catacombs scores from Death on the
card, and the human kept almost nothing in its pool.

Play weaknesses seen (none of them would have changed the result):

1. **Low conversion rate.** Cursed Forge cost 9 essences for 1 VP and was never used (every collect step the bot chose
   "turn this card" over paying 1 Death). Sacred Grove cost 12 essences for 2 VP. No Gold was saved for a monument
   (Hanging Gardens, Colossus): Cornucopia was always used for 3 essences, never for 1 Gold (still item 9.4). The bot
   ended with 10 idle essences and 3 Death on Cursed Skull (item 9.7).
2. **Pass timing (item 9.2).** In round 2 the bot held the first-player token and spent two more actions on a Magical
   Shard (+1.09 for passing against +0.62 for the Shard, then +1 Calm). The human passed first and took the token. It
   only counts at the victory check, so no points were lost, but it decides who acts first next round. `planner.rs`
   values the token at about 0.4 units mid-game.
3. **Pace (item 9.1).** `rounds left` is fixed at `(10 - best) / 2.2`. The bot showed 3.6 rounds left at the start of
   round 2; the game ended in round 4. The human's real pace was 2.5-3 VP a round.

Next steps, in order: derive pace from the rival's scoring card and pool; penalise idle essences near the end; a Gold
bonus when a monument is in reach; value passing first while holding the token. Expected effect: the bot scores about
6-7 instead of 4 in this game, but still loses the race.

Implemented (all in `Params`, 400 paired 2-player self-play games each, full defaults against the same bot with one
number switched off):

- `rate` (1.0): a player's pace is the larger of `pace` and the points its scoring cards make per round (`vp_rate`:
  essences collected onto a card that counts them, plus its free powers). A Catacombs with Death collect counts 2 a
  round. Switched off: 200-200.
- `idle` (0.4): pool essences lose up to this share of their worth as `left` falls below 2 rounds. Switched off: 201-199.
- `token_hold` (1.0): a pass is worth this much more to a player who holds the first player token while a rival has not
  passed. Switched off: 200-200.
- `gold_goal` (0, off): profit multiplier for monuments. At 0.3 it was 195-205 (noise, slightly worse), so it stays
  off until a human game shows it matters.

All differences are within noise (about +-25), as expected: the changes concern contested play against a person, which
self-play seldom produces. Not yet checked against the logged game; the next human game is the real test.
