//! Game flow: phases, turn order, round end and game end policies.

use serde::{Deserialize, Serialize};

use crate::{
    counter::Counter,
    id::{CounterId, PhaseId, PlayerId},
};

/// One step of a round. Interactive phases are played in turns; the others
/// resolve automatically (their `PhaseStarted` event is what abilities such as
/// "collect" hook into).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Phase {
    /// Phase id, defined by the game.
    pub id:          PhaseId,
    /// Whether players take turns in this phase.
    pub interactive: bool,
}

/// How the turn order evolves from one round to the next.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnOrderPolicy {
    /// The same order every round.
    #[default]
    Clockwise,
    /// Players who passed come first, in the order they passed (the first to
    /// pass becomes first player).
    PassOrder,
    /// The first player to pass starts the next round; everyone else keeps
    /// their clockwise order after them.
    FirstPasserStarts,
}

/// When an interactive phase is over.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RoundEndPolicy {
    /// As soon as one player passes.
    FirstPass,
    /// Once every player has passed.
    #[default]
    AllPassed,
}

/// Players in turn order, with who already passed this phase.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnOrder {
    order:   Vec<PlayerId>,
    policy:  TurnOrderPolicy,
    current: usize,
    passed:  Vec<PlayerId>,
}

impl TurnOrder {
    /// Creates a turn order; the first player starts.
    pub const fn new(order: Vec<PlayerId>, policy: TurnOrderPolicy) -> Self {
        Self {
            order,
            policy,
            current: 0,
            passed: Vec::new(),
        }
    }

    /// Players in the current order.
    pub fn order(&self) -> &[PlayerId] {
        &self.order
    }

    /// Players that passed, in the order they passed.
    pub fn passed(&self) -> &[PlayerId] {
        &self.passed
    }

    /// The player whose turn it is.
    pub fn current(&self) -> Option<PlayerId> {
        self.order.get(self.current).copied()
    }

    /// Whether the player passed already.
    pub fn has_passed(&self, player: PlayerId) -> bool {
        self.passed.contains(&player)
    }

    /// Records a pass (ignored when the player already passed).
    pub fn pass(&mut self, player: PlayerId) {
        if !self.has_passed(player) {
            self.passed.push(player);
        }
    }

    /// Whether everyone passed.
    pub const fn all_passed(&self) -> bool {
        !self.order.is_empty() && self.passed.len() == self.order.len()
    }

    /// Whether at least one player passed.
    pub const fn any_passed(&self) -> bool {
        !self.passed.is_empty()
    }

    /// Moves to the next player that has not passed. Returns `None` (and stays
    /// put) when everyone passed.
    pub fn advance(&mut self) -> Option<PlayerId> {
        let n = self.order.len();
        let next = (1..=n)
            .map(|step| (self.current + step) % n)
            .find(|i| !self.has_passed(self.order[*i]))?;
        self.current = next;
        self.current()
    }

    /// Reorders for the next round according to the policy. Call before
    /// [`TurnOrder::begin_phase`].
    pub fn reorder(&mut self) {
        match self.policy {
            TurnOrderPolicy::Clockwise => {},
            TurnOrderPolicy::PassOrder => {
                let mut order = self.passed.clone();
                order.extend(self.order.iter().filter(|p| !self.passed.contains(p)));
                self.order = order;
            },
            TurnOrderPolicy::FirstPasserStarts => {
                if let Some(first) = self.passed.first()
                    && let Some(at) = self.order.iter().position(|p| p == first)
                {
                    self.order.rotate_left(at);
                }
            },
        }
    }

    /// Forgets the passes and gives the turn to the first player.
    pub fn begin_phase(&mut self) {
        self.passed.clear();
        self.current = 0;
    }
}

/// When the game-end condition is evaluated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EndCheck {
    /// After every event: the first player to reach the target wins at once.
    AfterEvent,
    /// Only when a round ends (a victory check after each round).
    RoundEnd,
}

/// Win condition: reach `target` on `counter`; ties are broken by `tiebreak`
/// counters (higher wins).
///
/// Players still tied share the win. A counter owned by a team counts for each
/// of its members, so a winning team's members all win together.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameEndPolicy {
    /// The scoring counter (one per player or team).
    pub counter:  CounterId,
    /// Score needed to trigger the end.
    pub target:   i64,
    /// When to check.
    pub when:     EndCheck,
    /// Counters compared in order when scores tie.
    pub tiebreak: Vec<CounterId>,
}

impl GameEndPolicy {
    /// The winners, or `None` when nobody reached the target.
    pub fn winners(&self, counters: &[Counter], players: &[PlayerId]) -> Option<Vec<PlayerId>> {
        let value = |p: PlayerId, c: CounterId| {
            counters.iter().find(|x| x.id == c && x.owner.contains(p)).map_or(i64::MIN, |x| x.value)
        };
        let key = |p: PlayerId| -> Vec<i64> {
            std::iter::once(self.counter)
                .chain(self.tiebreak.iter().copied())
                .map(|c| value(p, c))
                .collect()
        };
        let candidates: Vec<_> =
            players.iter().copied().filter(|p| value(*p, self.counter) >= self.target).collect();
        let best = candidates.iter().map(|p| key(*p)).max()?;
        Some(candidates.into_iter().filter(|p| key(*p) == best).collect())
    }
}

/// The phase machine and turn bookkeeping.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flow {
    /// Phases of a round, in order.
    pub phases:           Vec<Phase>,
    /// Index of the running phase, `None` before the game started.
    pub phase:            Option<usize>,
    /// Turn order.
    pub turn_order:       TurnOrder,
    /// When an interactive phase ends.
    pub round_end:        RoundEndPolicy,
    /// If set, a turn ends automatically after this many actions (1 for
    /// one-action turns).
    pub actions_per_turn: Option<u32>,
    /// Actions taken in the current turn.
    pub actions_taken:    u32,
    /// Current round, starting at 1 once the game started.
    pub round:            u32,
    /// Win condition, if the game has one.
    pub game_end:         Option<GameEndPolicy>,
}

impl Flow {
    /// The player whose turn it is; `None` before the game starts and during
    /// automatic phases.
    pub fn active_player(&self) -> Option<PlayerId> {
        let interactive =
            self.phase.and_then(|i| self.phases.get(i)).is_some_and(|p| p.interactive);
        if interactive {
            self.turn_order.current()
        } else {
            None
        }
    }

    /// The id of the running phase.
    pub fn phase_id(&self) -> Option<PhaseId> {
        self.phase.and_then(|i| self.phases.get(i)).map(|p| p.id)
    }

    /// Whether the running interactive phase is over according to the round end
    /// policy.
    pub const fn phase_over(&self) -> bool {
        match self.round_end {
            RoundEndPolicy::FirstPass => self.turn_order.any_passed(),
            RoundEndPolicy::AllPassed => self.turn_order.all_passed(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zone::Owner;

    fn players(n: u8) -> Vec<PlayerId> {
        (0..n).map(PlayerId).collect()
    }

    #[test]
    fn clockwise_turns_cycle() {
        let mut t = TurnOrder::new(players(3), TurnOrderPolicy::Clockwise);
        assert_eq!(t.current(), Some(PlayerId(0)));
        assert_eq!(t.advance(), Some(PlayerId(1)));
        assert_eq!(t.advance(), Some(PlayerId(2)));
        assert_eq!(t.advance(), Some(PlayerId(0)));
    }

    #[test]
    fn passed_players_are_skipped() {
        let mut t = TurnOrder::new(players(3), TurnOrderPolicy::Clockwise);
        t.pass(PlayerId(1));
        assert_eq!(t.advance(), Some(PlayerId(2)));
        assert_eq!(t.advance(), Some(PlayerId(0)));
        assert_eq!(t.advance(), Some(PlayerId(2)));
    }

    #[test]
    fn advance_returns_none_once_everyone_passed() {
        let mut t = TurnOrder::new(players(2), TurnOrderPolicy::Clockwise);
        t.pass(PlayerId(0));
        t.pass(PlayerId(1));
        assert!(t.all_passed());
        assert_eq!(t.advance(), None);
    }

    #[test]
    fn passing_twice_counts_once() {
        let mut t = TurnOrder::new(players(2), TurnOrderPolicy::Clockwise);
        t.pass(PlayerId(0));
        t.pass(PlayerId(0));
        assert_eq!(t.passed(), &[PlayerId(0)]);
        assert!(!t.all_passed());
    }

    #[test]
    fn pass_order_makes_first_passer_first_player() {
        let mut t = TurnOrder::new(players(3), TurnOrderPolicy::PassOrder);
        t.pass(PlayerId(2));
        t.pass(PlayerId(0));
        t.pass(PlayerId(1));
        t.reorder();
        t.begin_phase();
        assert_eq!(t.order(), &[PlayerId(2), PlayerId(0), PlayerId(1)]);
        assert_eq!(t.current(), Some(PlayerId(2)));
        assert_eq!(t.passed(), &[] as &[PlayerId]);
    }

    #[test]
    fn clockwise_order_is_stable_across_rounds() {
        let mut t = TurnOrder::new(players(3), TurnOrderPolicy::Clockwise);
        t.pass(PlayerId(2));
        t.reorder();
        assert_eq!(t.order(), players(3).as_slice());
    }

    #[test]
    fn nobody_is_active_outside_interactive_phases() {
        let mut f = Flow {
            phases: vec![
                Phase {
                    id:          PhaseId(0),
                    interactive: false,
                },
                Phase {
                    id:          PhaseId(1),
                    interactive: true,
                },
            ],
            turn_order: TurnOrder::new(players(2), TurnOrderPolicy::Clockwise),
            ..Flow::default()
        };
        assert_eq!((f.active_player(), f.phase_id()), (None, None));
        f.phase = Some(0);
        assert_eq!((f.active_player(), f.phase_id()), (None, Some(PhaseId(0))));
        f.phase = Some(1);
        assert_eq!(f.active_player(), Some(PlayerId(0)));
    }

    #[test]
    fn round_end_policies() {
        let mut f = Flow {
            turn_order: TurnOrder::new(players(2), TurnOrderPolicy::Clockwise),
            round_end: RoundEndPolicy::FirstPass,
            ..Flow::default()
        };
        assert!(!f.phase_over());
        f.turn_order.pass(PlayerId(1));
        assert!(f.phase_over());
        f.round_end = RoundEndPolicy::AllPassed;
        assert!(!f.phase_over());
        f.turn_order.pass(PlayerId(0));
        assert!(f.phase_over());
    }

    fn vp(player: u8, value: i64) -> Counter {
        Counter::new(CounterId(0), Owner::Player(PlayerId(player)), value, 0, 100)
    }

    fn team_vp(players: Vec<u8>, value: i64) -> Counter {
        Counter::new(
            CounterId(0),
            Owner::Players(players.into_iter().map(PlayerId).collect()),
            value,
            0,
            100,
        )
    }

    fn tiebreak(player: u8, value: i64) -> Counter {
        Counter::new(CounterId(1), Owner::Player(PlayerId(player)), value, 0, 100)
    }

    fn policy() -> GameEndPolicy {
        GameEndPolicy {
            counter:  CounterId(0),
            target:   10,
            when:     EndCheck::RoundEnd,
            tiebreak: vec![CounterId(1)],
        }
    }

    #[test]
    fn nobody_wins_below_the_target() {
        assert_eq!(policy().winners(&[vp(0, 9), vp(1, 4)], &players(2)), None);
    }

    #[test]
    fn highest_score_at_or_above_target_wins() {
        let w = policy().winners(&[vp(0, 10), vp(1, 12)], &players(2));
        assert_eq!(w, Some(vec![PlayerId(1)]));
    }

    #[test]
    fn tied_scores_fall_back_to_the_tiebreak_counter() {
        let counters = [vp(0, 11), vp(1, 11), tiebreak(0, 3), tiebreak(1, 5)];
        assert_eq!(
            policy().winners(&counters, &players(2)),
            Some(vec![PlayerId(1)])
        );
    }

    #[test]
    fn team_counters_make_every_member_a_winner() {
        let counters = [team_vp(vec![0, 2], 12), vp(1, 9)];
        let w = policy().winners(&counters, &players(3));
        assert_eq!(w, Some(vec![PlayerId(0), PlayerId(2)]));
    }

    #[test]
    fn full_ties_share_the_win() {
        let counters = [vp(0, 11), vp(1, 11), tiebreak(0, 3), tiebreak(1, 3)];
        assert_eq!(
            policy().winners(&counters, &players(2)),
            Some(vec![PlayerId(0), PlayerId(1)])
        );
    }
}
