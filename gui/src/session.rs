//! A running game with people and bots at its seats, and the snapshots the
//! frontend renders.

use std::{collections::BTreeMap, path::Path};

use gamelib::{
    bot::Bot,
    effect::{Effect, OwnerRef, Target},
    engine::{Action, Engine, GameRules, RuleError, payments},
    id::{EntityId, PlayerId},
    pay,
    state::GameState,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    board::{BoardView, CardDto, ResAmount},
    describe::{Describer, plain, player_name},
    description::{GameDescription, Names, ScoreLine},
    record::{self, Recorder},
};

/// What a game provides to the GUI on top of its rules.
pub trait GameUi: Clone + Send + 'static {
    /// The engine rules this GUI plays.
    type Rules: GameRules + Clone + Send;

    /// A fresh copy of the rules.
    fn rules(&self) -> Self::Rules;

    /// The same game with the options (variants) a new game asks for. Games
    /// without variants keep the default, which ignores them.
    #[must_use]
    fn with_options(self, _options: &[String]) -> Self {
        self
    }

    /// Names, colors and card text.
    fn description(&self) -> GameDescription;

    /// An opponent for a bot seat.
    fn bot(&self, player: PlayerId, seed: u64) -> Box<dyn Bot + Send>;

    /// The dice rolled before the game to decide who starts, for the page to
    /// play back, if the game rolls any. `seed` is the game's seed.
    fn opening(&self, _players: u8, _seed: u64) -> Option<Opening> {
        None
    }

    /// The scoreboard right now.
    fn scores(&self, _state: &GameState) -> Vec<ScoreLine> {
        Vec::new()
    }

    /// A game-specific wording for one option of a decision, instead of the
    /// generic description (for example, saying what skipping would keep).
    fn option_label(
        &self,
        _state: &GameState,
        _pending: &gamelib::effect::PendingDecision,
        _option: &Effect,
    ) -> Option<String> {
        None
    }
}

/// One die rolled for a seat.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DieRoll {
    /// The seat that rolled.
    pub player: u8,
    /// The face that came up.
    pub value:  u8,
}

/// The dice that decided who starts, for the page to play back.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Opening {
    /// Each round of rolling; after the first only the seats tied for the
    /// highest roll roll again.
    pub rounds: Vec<Vec<DieRoll>>,
    /// The seat that starts.
    pub first:  u8,
}

/// Settings for a new game.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewGame {
    /// Number of seats.
    pub players: u8,
    /// Shuffle seed; random-looking default when absent.
    #[serde(default)]
    pub seed:    Option<u64>,
    /// Seats played by a person; the rest are bots. Defaults to seat 0.
    #[serde(default)]
    pub humans:  Option<Vec<u8>>,
    /// Game-specific options (variants), by the ids the game lists in its
    /// description; unknown ones are ignored.
    #[serde(default)]
    pub options: Vec<String>,
}

/// An action the viewer may take now. `index` is what to send back.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionView {
    /// Position in the legal action list of this snapshot.
    pub index:         usize,
    /// `pass`, `end_turn`, `buy`, `activate`, `place`, `discard` or `decide`.
    pub kind:          String,
    /// The card it concerns, if any.
    pub entity:        Option<u32>,
    /// Which ability of that card (`activate`).
    pub ability:       Option<usize>,
    /// What it does, in words.
    pub label:         String,
    /// The same with its terms marked (see `describe::plain`), for a page that
    /// underlines and explains them.
    pub markup:        String,
    /// What it costs, in words.
    pub detail:        String,
    /// The same with its terms marked (see `describe::plain`).
    pub detail_markup: String,
    /// For a decision that is only a mix of resources: what it gives or takes.
    #[serde(default)]
    pub mix:           Option<Mix>,
    /// The cards this action is about (a decision's options show them), as the
    /// viewer may see them.
    pub cards:         Vec<CardDto>,
    /// For a decision option that uses an ability of the card `entity`: what
    /// that ability does, with marked terms. A page can then offer the option
    /// as a button on the card instead of in a dialog.
    #[serde(default)]
    pub short:         String,
    /// The decision option that does nothing (declining, or ending a step).
    #[serde(default)]
    pub skip:          bool,
}

/// What a decision option gives or takes, when it is just a mix of resources.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mix {
    /// `gain` (you obtain them) or `pay` (you give them up).
    pub direction: String,
    /// How many of each resource.
    pub amounts:   Vec<ResAmount>,
}

/// The resources a decision option gives or takes, if it does nothing else.
fn mix_of(effect: &Effect) -> Option<Mix> {
    const fn part(effect: &Effect) -> Option<(bool, ResAmount)> {
        match effect {
            Effect::Gain {
                to: Target::Controller,
                res,
                n,
            } => Some((
                true,
                ResAmount {
                    res: res.0,
                    n:   *n,
                },
            )),
            Effect::Pay { res, n } => Some((
                false,
                ResAmount {
                    res: res.0,
                    n:   *n,
                },
            )),
            _ => None,
        }
    }
    let parts: Vec<_> = match effect {
        Effect::Sequence(steps) => steps.iter().map(part).collect::<Option<Vec<_>>>()?,
        other => vec![part(other)?],
    };
    let gain = parts.first()?.0;
    if parts.iter().any(|p| p.0 != gain) {
        return None;
    }
    Some(Mix {
        direction: if gain { "gain" } else { "pay" }.to_owned(),
        amounts:   parts.into_iter().map(|p| p.1).collect(),
    })
}

/// One line of the game log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogLine {
    /// The seat the line is about, if any.
    pub player: Option<u8>,
    /// The text.
    pub text:   String,
    /// The text with its terms marked (see `describe::plain`).
    pub markup: String,
}

/// Everything the frontend needs to draw one player's screen.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    /// Changes whenever the game state does; actions must quote it.
    pub revision: u64,
    /// The board as the viewer sees it.
    pub board:    BoardView,
    /// What the viewer may do now (empty when it is not their move).
    pub actions:  Vec<ActionView>,
    /// Seats that have a legal action now.
    pub acting:   Vec<u8>,
    /// Recent events, oldest first.
    pub log:      Vec<LogLine>,
    /// The dice that decided who starts, if the game rolled any.
    #[serde(default)]
    pub opening:  Option<Opening>,
    /// The scoreboard.
    pub scores:   Vec<ScoreLine>,
    /// The winners once the game is over.
    pub winners:  Option<Vec<u8>>,
    /// The planned moves while the viewer is planning; everything else in the
    /// snapshot then shows the plan's what-if game, not the real one.
    #[serde(default)]
    pub plan:     Option<PlanView>,
    /// Something to tell the viewer once (how far a plan could be played).
    #[serde(default)]
    pub notice:   Option<String>,
    /// Whether the notice is a warning (the other player interfered).
    #[serde(default)]
    pub alert:    bool,
    /// Whether the next player moves by itself after a move.
    #[serde(default)]
    pub auto:          bool,
    /// Whether the next player waits for the viewer to press "Next".
    #[serde(default)]
    pub awaiting_next: bool,
    /// Whether the viewer can take back what they did this turn.
    #[serde(default)]
    pub can_undo:      bool,
}

/// One planned move, as the page lists it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanStepView {
    /// What the move does, in words.
    pub label:  String,
    /// What it costs, in words.
    pub detail: String,
    /// Whether it is applied in the what-if game shown (undone moves stay in
    /// the list until a new move replaces them).
    pub done:   bool,
}

/// The plan of a viewer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanView {
    /// The planned moves, in order.
    pub steps:    Vec<PlanStepView>,
    /// Whether a move can be taken back.
    pub can_undo: bool,
    /// Whether an undone move can be put back.
    pub can_redo: bool,
}

/// A planned move.
#[derive(Clone, Debug)]
struct PlanStep {
    action: Action,
    label:  String,
    detail: String,
}

/// A what-if copy of the game in which one person tries moves out. Nothing
/// in it touches the real game: the rivals never act (the turn comes straight
/// back to the planner; a decision of theirs takes its first option), and the
/// piles nobody may look at are shuffled first, so planning reveals nothing
/// (a card "drawn" while planning is a guess).
struct Plan<R: GameRules> {
    viewer: PlayerId,
    /// Where the plan starts.
    base:   GameState,
    /// The game after the first `done` steps.
    engine: Engine<R>,
    steps:  Vec<PlanStep>,
    done:   usize,
}

/// Steps a plan's what-if game may take by itself after a move; a bug guard.
const MAX_PLAN_STEPS: usize = 500;

/// Most steps a bot run may take; a bug guard.
const MAX_BOT_STEPS: usize = 100_000;
/// Log lines sent with a snapshot.
const LOG_LINES: usize = 60;

/// A running game.
pub struct Session<G: GameUi> {
    ui:       G,
    engine:   Engine<G::Rules>,
    humans:   Vec<PlayerId>,
    bots:     BTreeMap<PlayerId, Box<dyn Bot + Send>>,
    names:    Names,
    revision: u64,
    opening: Option<Opening>,
    recorder: Option<Recorder>,
    plan:     Option<Plan<G::Rules>>,
    notice:   Option<String>,
    /// Whether the notice is a warning (shown in red).
    notice_alert: bool,
    /// Whether the bots answer at once after a person's move. When not, they
    /// wait for [`Session::next`] and the person may take the move back.
    auto:     bool,
    /// A bot is due to move but waits for [`Session::next`].
    awaiting: bool,
    /// How many actions of the game were played when the person's turn began
    /// (what [`Session::undo`] goes back to).
    checkpoint: usize,
}

impl<G: GameUi> Session<G> {
    /// Starts a game and lets the bots play until a person must act.
    pub fn new(ui: G, game: &NewGame, description: &GameDescription) -> Result<Self, String> {
        Self::with_log(ui, game, description, None)
    }

    /// The same, writing the game to a new file in `log_dir` (see
    /// [`crate::record`]). A log that cannot be created does not stop the game.
    pub fn with_log(
        ui: G,
        game: &NewGame,
        description: &GameDescription,
        log_dir: Option<&Path>,
    ) -> Result<Self, String> {
        if game.players < description.min_players || game.players > description.max_players {
            return Err(format!(
                "{} to {} players, not {}",
                description.min_players, description.max_players, game.players
            ));
        }
        let ui = ui.with_options(&game.options);
        let seed = game.seed.unwrap_or(1);
        let humans: Vec<PlayerId> =
            game.humans.clone().unwrap_or_else(|| vec![0]).into_iter().map(PlayerId).collect();
        if humans.iter().any(|h| h.0 >= game.players) {
            return Err("a human seat is out of range".into());
        }
        let engine = Engine::new(ui.rules(), game.players, seed)
            .map_err(|e| format!("setup failed: {e:?}"))?;
        let bots = (0..game.players)
            .map(PlayerId)
            .filter(|p| !humans.contains(p))
            .map(|p| {
                (
                    p,
                    ui.bot(
                        p,
                        seed.wrapping_add(u64::from(p.0)).wrapping_mul(31).wrapping_add(7),
                    ),
                )
            })
            .collect();
        let ui_opening = ui.opening(game.players, seed);
        let mut session = Self {
            ui,
            engine,
            humans,
            bots,
            names: Names::new(description),
            revision: 0,
            opening: ui_opening,
            recorder: log_dir.and_then(|dir| Recorder::create(dir, seed).ok()),
            plan: None,
            notice: None,
            notice_alert: false,
            auto: true,
            awaiting: false,
            checkpoint: 0,
        };
        session.record_start(&description.title, game.players);
        session.run_bots()?;
        session.checkpoint = session.engine.log().len();
        Ok(session)
    }

    fn clear_notice(&mut self) {
        self.notice = None;
        self.notice_alert = false;
    }

    /// The current revision.
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// The engine, for tests and tools.
    pub const fn engine(&self) -> &Engine<G::Rules> {
        &self.engine
    }

    /// Lets bots act until a person must act or the game is over. Without
    /// "auto next player" a bot waits for [`Session::next`] instead.
    pub fn run_bots(&mut self) -> Result<(), String> {
        self.advance(self.auto)
    }

    /// Whether the bots move by themselves after a person's move.
    pub const fn auto(&self) -> bool {
        self.auto
    }

    /// Turns "auto next player" on or off. Turning it on lets a waiting bot
    /// move now.
    pub fn set_auto(&mut self, auto: bool) -> Result<(), String> {
        self.auto = auto;
        // While planning nothing real may move: the bots wait until the plan
        // is left.
        if auto && self.awaiting && self.plan.is_none() {
            self.advance(true)?;
        }
        Ok(())
    }

    /// Lets the next player (a bot) move; the person's turn is over.
    pub fn next(&mut self) -> Result<(), String> {
        if self.plan.is_some() {
            return Err("leave planning first".into());
        }
        if !self.awaiting {
            return Err("nobody is waiting".into());
        }
        self.clear_notice();
        self.advance(true)
    }

    /// Whether the person can take back what they did since their turn began.
    pub fn can_undo(&self) -> bool {
        !self.auto && self.engine.winners().is_none() && self.engine.log().len() > self.checkpoint
    }

    /// Takes back every move of the person since their turn began.
    pub fn undo(&mut self) -> Result<(), String> {
        if !self.can_undo() {
            return Err("nothing to undo".into());
        }
        if self.plan.is_some() {
            return Err("leave planning first".into());
        }
        self.clear_notice();
        let kept = self.checkpoint;
        let removed = self.engine.log().len() - kept;
        let engine = Engine::replay(
            self.ui.rules(),
            u8::try_from(self.engine.players().len()).map_err(|_| "too many players")?,
            self.engine.seed(),
            &self.engine.log()[..kept],
        )
        .map_err(|e| format!("undo failed: {e:?}"))?;
        self.engine = engine;
        self.awaiting = false;
        self.revision += 1;
        if let Some(recorder) = &mut self.recorder {
            // Moves after `to` in the file were taken back.
            recorder.write(&json!({
                "type": "undo",
                "time": record::time(),
                "removed": removed,
                "to": kept,
            }));
        }
        Ok(())
    }

    fn advance(&mut self, permit: bool) -> Result<(), String> {
        self.awaiting = false;
        let mut moved = false;
        for _ in 0..MAX_BOT_STEPS {
            if self.engine.winners().is_some() {
                return Ok(());
            }
            let players = self.engine.players().to_vec();
            let mut person_waits = false;
            let mut acted = false;
            for p in players {
                let legal = self.engine.legal_actions(p);
                if legal.is_empty() {
                    continue;
                }
                let Some(bot) = self.bots.get_mut(&p) else {
                    person_waits = true;
                    continue;
                };
                if !permit {
                    // A bot is due, but waits for "Next" (unless a person can
                    // still act: then it is theirs to go on).
                    self.awaiting = !person_waits;
                    return Ok(());
                }
                let choice = bot
                    .choose_with_state(self.engine.state(), &self.engine.view(p), &legal)
                    .min(legal.len() - 1);
                let note = bot.explain();
                let before = self.before_action(p, choice, &legal[choice]);
                self.engine
                    .apply(p, legal[choice].clone())
                    .map_err(|e| format!("a bot move failed: {e:?}"))?;
                self.revision += 1;
                self.record_action(p, &legal[choice], before, note);
                acted = true;
                moved = true;
                break;
            }
            if !acted {
                return if person_waits {
                    if moved {
                        self.checkpoint = self.engine.log().len();
                    }
                    Ok(())
                } else {
                    Err("nobody can act".into())
                };
            }
        }
        Err("the bots did not stop".into())
    }

    /// A person plays the `index`-th legal action of the snapshot with this
    /// revision; then the bots answer. While that person is planning, the
    /// move is only tried out in their plan.
    pub fn act(&mut self, player: PlayerId, revision: u64, index: usize) -> Result<(), String> {
        if !self.humans.contains(&player) {
            return Err(format!("{} is played by a bot", player_name(player)));
        }
        if revision != self.revision {
            return Err("the game moved on; refresh".into());
        }
        self.clear_notice();
        if self.plan.is_some() {
            return self.plan_act(player, index);
        }
        self.act_for_real(player, index)
    }

    fn act_for_real(&mut self, player: PlayerId, index: usize) -> Result<(), String> {
        self.apply_for_real(player, index)?;
        self.run_bots()
    }

    /// Plays a person's move without letting the bots answer.
    fn apply_for_real(&mut self, player: PlayerId, index: usize) -> Result<(), String> {
        let legal = self.engine.legal_actions(player);
        let action = legal.get(index).ok_or("no such action")?.clone();
        let before = self.before_action(player, index, &action);
        self.engine.apply(player, action.clone()).map_err(|e: RuleError| format!("{e:?}"))?;
        self.revision += 1;
        self.record_action(player, &action, before, None);
        Ok(())
    }

    /// What a player holds that a rival can take away: each essence in the
    /// pool, the cards in hand and the cards in play.
    fn holdings(&self, player: PlayerId) -> Vec<u32> {
        let state = self.engine.state();
        let mut out = Vec::new();
        if let Ok(pool) = state.player_pool(player) {
            let mut kinds: Vec<_> = pool.iter().map(|(res, _)| res).collect();
            kinds.sort_unstable();
            out.extend(kinds.into_iter().map(|res| pool.finite(res).unwrap_or(0)));
        }
        if let Some(rules) = &state.card_rules {
            for kind in [rules.hand, rules.tableau] {
                let count = state
                    .zone_id(&gamelib::zone::Owner::Player(player), kind)
                    .and_then(|z| state.zone(z))
                    .map_or(0, |z| z.entities().len());
                out.push(u32::try_from(count).unwrap_or(u32::MAX));
            }
        }
        out
    }

    /// The game a snapshot shows: the plan's what-if game while somebody is
    /// planning, the real one otherwise.
    fn shown(&self) -> &Engine<G::Rules> {
        self.plan.as_ref().map_or(&self.engine, |plan| &plan.engine)
    }

    /// Whether somebody is planning.
    pub const fn planning(&self) -> bool {
        self.plan.is_some()
    }

    /// Lets a plan's what-if game run until the planner can move again: a
    /// rival's decision takes its first option, and a rival's turn is skipped.
    fn plan_settle(&self, mut engine: Engine<G::Rules>, viewer: PlayerId) -> Engine<G::Rules> {
        for _ in 0..MAX_PLAN_STEPS {
            if engine.winners().is_some() {
                break;
            }
            if let Some(pending) = engine.pending() {
                if pending.waiting_for(viewer) {
                    break;
                }
                let Some(&rival) = pending.players.iter().find(|p| pending.waiting_for(**p)) else {
                    break;
                };
                let Some(first) = engine.legal_actions(rival).into_iter().next() else {
                    break;
                };
                if engine.apply(rival, first).is_err() {
                    break;
                }
                continue;
            }
            let order = &engine.state().flow.turn_order;
            match engine.current_player() {
                Some(active) if active != viewer && !order.has_passed(viewer) => {
                    // The rivals do nothing in a plan: the turn comes back.
                    let mut state = engine.state().clone();
                    let seats = state.flow.turn_order.order().len();
                    for _ in 0..seats {
                        if state.flow.turn_order.advance() == Some(viewer) {
                            break;
                        }
                    }
                    state.flow.actions_taken = 0;
                    engine = Engine::from_state(self.ui.rules(), state);
                },
                _ => break,
            }
        }
        engine
    }

    /// The plan's game after its first `done` steps, replayed from its start.
    /// A step that no longer applies ends the plan there.
    fn plan_replay(&self, plan: &mut Plan<G::Rules>) {
        let mut engine = self.plan_settle(
            Engine::from_state(self.ui.rules(), plan.base.clone()),
            plan.viewer,
        );
        let mut applied = 0;
        for step in &plan.steps[..plan.done] {
            if engine.apply(plan.viewer, step.action.clone()).is_err() {
                break;
            }
            engine = self.plan_settle(engine, plan.viewer);
            applied += 1;
        }
        if applied < plan.done {
            plan.steps.truncate(applied);
            plan.done = applied;
        }
        plan.engine = engine;
    }

    /// Starts planning for a person: from now on their moves are tried out in
    /// a what-if copy of the game and listed, until the plan is played
    /// ([`Session::plan_run`]) or dropped ([`Session::plan_stop`]).
    pub fn plan_start(&mut self, player: PlayerId) -> Result<(), String> {
        if !self.humans.contains(&player) {
            return Err(format!("{} is played by a bot", player_name(player)));
        }
        if self.plan.is_some() {
            return Err("a plan is already open".into());
        }
        if self.engine.winners().is_some() {
            return Err("the game is over".into());
        }
        if self.awaiting {
            return Err("press Next first: a bot is due to move".into());
        }
        self.open_plan(player, Vec::new());
        self.clear_notice();
        Ok(())
    }

    /// Opens a plan on the real game as it is now, with `steps` already on
    /// its list (those that no longer apply are dropped).
    fn open_plan(&mut self, player: PlayerId, steps: Vec<PlanStep>) {
        let mut base = self.engine.state().clone();
        // Nobody may learn what a pile holds by planning a draw.
        let hidden: Vec<_> = base
            .zones()
            .filter(|z| z.visibility == gamelib::zone::Visibility::Hidden)
            .map(|z| z.id)
            .collect();
        for zone in hidden {
            let _ = base.shuffle(zone);
        }
        base.clear_events();
        let engine = Engine::from_state(self.ui.rules(), base.clone());
        let done = steps.len();
        let mut plan = Plan {
            viewer: player,
            base,
            engine,
            steps,
            done,
        };
        self.plan_replay(&mut plan);
        self.plan = Some(plan);
        self.revision += 1;
    }

    /// Drops the plan; the real game is shown again, untouched.
    pub fn plan_stop(&mut self) {
        if self.plan.take().is_some() {
            self.revision += 1;
        }
        self.clear_notice();
        // Bots that were held back while planning (auto turned on) move now.
        if self.auto && self.awaiting {
            let _ = self.advance(true);
        }
    }

    /// Tries a move out in the plan and adds it to the list (replacing the
    /// moves that were undone).
    fn plan_act(&mut self, player: PlayerId, index: usize) -> Result<(), String> {
        let Some(plan) = &self.plan else {
            return Err("no plan is open".into());
        };
        if plan.viewer != player {
            return Err(format!("{} is planning", player_name(plan.viewer)));
        }
        let action = plan.engine.legal_actions(player).get(index).ok_or("no such action")?.clone();
        // The words are read from the plan's game, before the move changes it.
        let view = self.action_view(player, index, &action);
        let Some(mut plan) = self.plan.take() else {
            return Err("no plan is open".into());
        };
        let outcome = plan.engine.apply(player, action.clone()).map(|_| ());
        if outcome.is_ok() {
            plan.steps.truncate(plan.done);
            plan.steps.push(PlanStep {
                action,
                label: view.label,
                detail: view.detail,
            });
            plan.done += 1;
            // Replaying keeps one way of getting to a plan's game.
            self.plan_replay(&mut plan);
        }
        self.plan = Some(plan);
        self.revision += 1;
        outcome.map_err(|e| format!("{e:?}"))
    }

    /// Takes the last planned move back (it can be put back with
    /// [`Session::plan_redo`]).
    pub fn plan_undo(&mut self) -> Result<(), String> {
        self.plan_move(false)
    }

    /// Puts the last undone move back.
    pub fn plan_redo(&mut self) -> Result<(), String> {
        self.plan_move(true)
    }

    fn plan_move(&mut self, forward: bool) -> Result<(), String> {
        let Some(mut plan) = self.plan.take() else {
            return Err("no plan is open".into());
        };
        let possible = if forward {
            plan.done < plan.steps.len()
        } else {
            plan.done > 0
        };
        if possible {
            plan.done = if forward {
                plan.done + 1
            } else {
                plan.done - 1
            };
            self.plan_replay(&mut plan);
            self.revision += 1;
        }
        self.plan = Some(plan);
        self.clear_notice();
        if possible {
            Ok(())
        } else if forward {
            Err("nothing to redo".into())
        } else {
            Err("nothing to undo".into())
        }
    }

    /// Plays the planned moves for real, one after the other, with the bots
    /// answering in between, and stops at the first one that is no longer
    /// possible as planned (the rivals may have changed things). The plan is
    /// closed; the snapshot's `notice` says how far it got.
    pub fn plan_run(&mut self, player: PlayerId) -> Result<(), String> {
        let Some(plan) = self.plan.take() else {
            return Err("no plan is open".into());
        };
        if plan.viewer != player {
            let owner = plan.viewer;
            self.plan = Some(plan);
            return Err(format!("{} is planning", player_name(owner)));
        }
        self.revision += 1;
        let steps = &plan.steps[..plan.done];
        let mut count = 0;
        let mut stopped = None;
        let mut interfered = false;
        for step in steps {
            if self.engine.winners().is_some() {
                stopped = Some("the game is over".to_owned());
                break;
            }
            let legal = self.engine.legal_actions(player);
            // The same move in the same words: the way of paying is a number
            // that can mean something else once the pool changed.
            let same = |index: usize, exact: bool| {
                let view = self.action_view(player, index, &legal[index]);
                view.label == step.label
                    && view.detail == step.detail
                    && (!exact || legal[index] == step.action)
            };
            let found = (0..legal.len())
                .find(|i| same(*i, true))
                .or_else(|| (0..legal.len()).find(|i| same(*i, false)));
            let Some(index) = found else {
                stopped = Some(format!(
                    "\u{201c}{}\u{201d} is not possible now",
                    step.label
                ));
                break;
            };
            if let Err(e) = self.apply_for_real(player, index) {
                stopped = Some(e);
                break;
            }
            count += 1;
            let mine = self.holdings(player);
            if let Err(e) = self.run_bots() {
                stopped = Some(e);
                break;
            }
            // The answer of a bot took something from the planner (a
            // Dragon's attack, a card discarded): the rest of the plan may no
            // longer make sense. Stop and keep it.
            let now = self.holdings(player);
            if now.len() == mine.len() && now.iter().zip(&mine).any(|(a, b)| a < b) {
                interfered = true;
                break;
            }
        }
        if interfered {
            let rest = plan.steps[count..plan.done].to_vec();
            let kept = rest.len();
            self.open_plan(player, rest);
            let alive = self.plan.as_ref().map_or(0, |p| p.steps.len());
            if alive == 0 {
                self.plan = None;
            }
            self.notice_alert = true;
            self.notice = Some(format!(
                "The other player interfered. Played {count} of {} planned moves, then stopped.{}",
                steps.len(),
                if alive > 0 {
                    format!(" The other {alive} move(s) are kept in your plan: check them.")
                } else if kept > 0 {
                    " The rest of the plan is no longer possible.".to_owned()
                } else {
                    String::new()
                }
            ));
            return Ok(());
        }
        self.notice_alert = false;
        self.notice = Some(stopped.map_or_else(
            || format!("Played all {count} planned moves."),
            |why| {
                format!(
                    "Played {count} of {} planned moves, then stopped: {why}.",
                    steps.len()
                )
            },
        ));
        Ok(())
    }

    /// The file this game is written to, if it is being logged.
    pub fn log_path(&self) -> Option<&Path> {
        self.recorder.as_ref().map(Recorder::path)
    }

    fn scores_json(&self) -> serde_json::Value {
        let mut out = serde_json::Map::new();
        for line in self.ui.scores(self.engine.state()) {
            out.insert(line.player.to_string(), json!(line.value));
        }
        serde_json::Value::Object(out)
    }

    fn record_start(&mut self, title: &str, players: u8) {
        if self.recorder.is_none() {
            return;
        }
        let scores = self.scores_json();
        let line = json!({
            "type": "start",
            "game": title,
            "time": record::time(),
            "seed": self.engine.seed(),
            "players": players,
            "humans": record::seats(&self.humans),
        });
        if let Some(recorder) = &mut self.recorder {
            recorder.write(&line);
            recorder.round(self.engine.state(), &self.names, &scores);
        }
    }

    /// What is only known before a move is applied: its words, and where the
    /// game was. `None` when the game is not logged.
    fn before_action(
        &self,
        player: PlayerId,
        index: usize,
        action: &Action,
    ) -> Option<serde_json::Value> {
        self.recorder.as_ref()?;
        let view = self.action_view(player, index, action);
        let flow = &self.engine.state().flow;
        Some(json!({
            "round": flow.round,
            "phase": flow.phase_id().map(|p| self.names.phase(p)),
            "label": view.label,
            "detail": view.detail,
        }))
    }

    /// Writes one move, then the new round if one began, then the end.
    fn record_action(
        &mut self,
        player: PlayerId,
        action: &Action,
        before: Option<serde_json::Value>,
        note: Option<String>,
    ) {
        let Some(before) = before else {
            return;
        };
        let state = self.engine.state();
        let scores = self.scores_json();
        let mut line = json!({
            "type": "action",
            "n": self.engine.log().len(),
            "time": record::time(),
            "player": player.0,
            "human": self.humans.contains(&player),
            "action": action,
            "scores": scores,
            "pools": record::pools(state, &self.names),
        });
        if let (Some(line), Some(before)) = (line.as_object_mut(), before.as_object()) {
            for (key, value) in before {
                line.insert(key.clone(), value.clone());
            }
            if let Some(note) = note {
                line.insert("bot".to_owned(), json!(note));
            }
        }
        let end = self.engine.winners().map(|winners| {
            json!({
                "type": "end",
                "time": record::time(),
                "winners": record::seats(winners),
                "round": state.flow.round,
                "actions": self.engine.log().len(),
                "scores": scores,
                "players": record::positions(state, &self.names),
            })
        });
        if let Some(recorder) = &mut self.recorder {
            recorder.write(&line);
            match end {
                Some(end) => recorder.write(&end),
                None => recorder.round(self.engine.state(), &self.names, &scores),
            }
        }
    }

    /// The top cards of a pile in the order `perm` would leave them (top
    /// first), revealed: the player choosing the order is looking at them.
    fn looked_at(
        &self,
        player: PlayerId,
        owner: &OwnerRef,
        zone: gamelib::id::ZoneKind,
        perm: &[usize],
    ) -> Option<(Vec<CardDto>, Vec<String>)> {
        let state = self.shown().state();
        let owner = gamelib::effect::Ctx::new(None, player).owner(owner);
        let pile = state.zone(state.zone_id(&owner, zone).ok()?).ok()?;
        let top_first: Vec<EntityId> = pile.entities().iter().rev().copied().collect();
        let mut cards = Vec::new();
        let mut names = Vec::new();
        for from in perm {
            let entity = state.entity(*top_first.get(*from)?).ok()?;
            names.push(self.names.card(entity.def));
            cards.push(CardDto {
                id:        entity.id.0,
                def:       Some(entity.def.0),
                face_up:   true,
                exhausted: entity.exhausted,
                pool:      Vec::new(),
            });
        }
        Some((cards, names))
    }

    fn describer(&self, viewer: PlayerId) -> Describer<'_> {
        Describer::for_viewer(self.shown().state(), &self.names, viewer)
    }

    fn card_cost(&self, entity: EntityId) -> Option<gamelib::cost::Cost> {
        let state = self.shown().state();
        state
            .entity(entity)
            .ok()
            .and_then(|e| state.defs.get(&e.def))
            .and_then(|d| d.cost.clone())
    }

    fn action_view(&self, player: PlayerId, index: usize, action: &Action) -> ActionView {
        let d = self.describer(player).with_markup();
        let state = self.shown().state();
        let mut view = ActionView {
            index,
            kind: String::new(),
            entity: None,
            ability: None,
            label: String::new(),
            markup: String::new(),
            detail: String::new(),
            detail_markup: String::new(),
            mix: None,
            cards: Vec::new(),
            short: String::new(),
            skip: false,
        };
        match action {
            Action::Pass => {
                "pass".clone_into(&mut view.kind);
                "Pass".clone_into(&mut view.label);
            },
            Action::EndTurn => {
                "end_turn".clone_into(&mut view.kind);
                "End turn".clone_into(&mut view.label);
            },
            Action::Buy { entity, pay } => {
                "buy".clone_into(&mut view.kind);
                view.entity = Some(entity.0);
                view.label = format!("Claim {}", d.entity(*entity));
                let cost = self.card_cost(*entity);
                if let Some(p) = payments(state, player, None, cost.as_ref(), *entity).get(*pay) {
                    view.detail = d.payment(p);
                }
            },
            Action::Place { entity, pay } => {
                "place".clone_into(&mut view.kind);
                view.entity = Some(entity.0);
                view.label = format!("Place {}", d.entity(*entity));
                if let Some(p) = pay::place_payments(state, player, *entity, None).get(*pay) {
                    view.detail = d.payment(p);
                }
            },
            Action::Discard { entity } => {
                "discard".clone_into(&mut view.kind);
                view.entity = Some(entity.0);
                view.label = format!("Discard {}", d.entity(*entity));
            },
            Action::Activate {
                entity,
                ability,
                pay,
            } => {
                "activate".clone_into(&mut view.kind);
                view.entity = Some(entity.0);
                view.ability = Some(*ability);
                view.label = format!("{}: {}", d.entity(*entity), d.ability(*entity, *ability));
                let ability_cost = state
                    .entity(*entity)
                    .ok()
                    .and_then(|e| state.defs.get(&e.def))
                    .and_then(|def| def.abilities.get(*ability))
                    .and_then(|a| a.cost.clone());
                if let Some(p) =
                    payments(state, player, Some(*entity), ability_cost.as_ref(), *entity).get(*pay)
                {
                    view.detail = d.payment(p);
                }
            },
            Action::Decide(choice) => {
                "decide".clone_into(&mut view.kind);
                let option = self.shown().pending().and_then(|p| p.options.get(*choice));
                view.label =
                    option.map_or_else(|| format!("Option {}", choice + 1), |o| d.effect(o));
                view.cards = option.map(|o| d.cards_in(o)).unwrap_or_default();
                view.mix = option.and_then(mix_of);
                view.skip = option == Some(&Effect::Ignore);
                if let (Some(pending), Some(o)) = (self.shown().pending(), option)
                    && let Some(label) = self.ui.option_label(state, pending, o)
                {
                    view.label = label;
                }
                Self::card_option(&d, option, &mut view);
                // Looking at the top cards of a pile: the option shows them,
                // top first, in the order it would put them back.
                if let Some(Effect::Permute { owner, zone, perm }) = option
                    && let Some((cards, names)) = self.looked_at(player, owner, *zone, perm)
                {
                    view.cards = cards;
                    view.label = format!("put back, top first: {}", names.join(", "));
                }
            },
        }
        // Labels were written with marked terms; keep both forms.
        view.markup = std::mem::take(&mut view.label);
        view.label = plain(&view.markup);
        view.detail_markup = std::mem::take(&mut view.detail);
        view.detail = plain(&view.detail_markup);
        view
    }

    /// A decision option that uses an ability of one of the viewer's cards
    /// belongs to that card: the view names it and says what the ability does.
    fn card_option(d: &Describer<'_>, option: Option<&Effect>, view: &mut ActionView) {
        let used = match option {
            Some(Effect::Sequence(steps)) => steps.first(),
            other => other,
        };
        if let Some(Effect::Use {
            source,
            ability,
            payment,
        }) = used
        {
            view.entity = Some(source.0);
            view.ability = Some(*ability);
            view.short = d.ability(*source, *ability);
            view.detail = d.payment(payment);
        }
    }

    /// Everything the frontend needs for `viewer`'s screen.
    pub fn snapshot(&self, viewer: PlayerId) -> Snapshot {
        let state = self.shown().state();
        let view = self.shown().view(viewer);
        let flow = &state.flow;
        let board = BoardView::new(
            &view,
            flow.turn_order.order(),
            flow.turn_order.passed(),
            &self.humans,
        );
        let actions = self
            .shown()
            .legal_actions(viewer)
            .iter()
            .enumerate()
            .map(|(i, a)| self.action_view(viewer, i, a))
            .collect();
        let acting = self
            .shown()
            .players()
            .iter()
            .filter(|p| !self.shown().legal_actions(**p).is_empty())
            .map(|p| p.0)
            .collect();
        let d = self.describer(viewer).with_markup();
        let mut log: Vec<LogLine> = self
            .shown()
            .events()
            .iter()
            .rev()
            .filter_map(|e| d.event(viewer, e))
            .take(LOG_LINES)
            .map(|(player, markup)| LogLine {
                player,
                text: plain(&markup),
                markup,
            })
            .collect();
        log.reverse();
        Snapshot {
            revision: self.revision,
            board,
            actions,
            acting,
            log,
            opening: self.opening.clone(),
            scores: self.ui.scores(state),
            winners: self.shown().winners().map(|w| w.iter().map(|p| p.0).collect()),
            plan: self.plan.as_ref().map(|plan| PlanView {
                steps:    plan
                    .steps
                    .iter()
                    .enumerate()
                    .map(|(i, step)| PlanStepView {
                        label:  step.label.clone(),
                        detail: step.detail.clone(),
                        done:   i < plan.done,
                    })
                    .collect(),
                can_undo: plan.done > 0,
                can_redo: plan.done < plan.steps.len(),
            }),
            notice: self.notice.clone(),
            alert: self.notice_alert,
            auto: self.auto,
            awaiting_next: self.awaiting && self.plan.is_none(),
            can_undo: self.can_undo() && self.plan.is_none(),
        }
    }
}
