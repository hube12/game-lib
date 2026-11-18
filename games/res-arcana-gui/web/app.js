// The Res Arcana page: lays the shared components out for this game.
import { call } from './gamelib-gui/transport.js';
import { h, clear } from './gamelib-gui/dom.js';
import { Model, findZone, zoneView, decisionDialog, decidedOnCards, logView, scoreboard, poolView, hidePopup, playOpening } from './gamelib-gui/components.js';

// Zone roles, phases and counters of this game (the same ids the Rust side names).
const ZONE = { HAND: 1, TABLEAU: 3, MONUMENT_DECK: 10, PLACES: 11, MONUMENT_ROW: 12, ITEMS: 13, CANDIDATES: 14, DRAFT: 17, DECK: 0 };
const COUNTER = { FIRST_TOKEN: 2 };

const app = document.getElementById('app');
let model = null;
let snapshot = null;
let viewer = 0;
let error = null;

const defaults = { players: 2, seed: 1, german: false, draft: false, difficulty: 5 };
function loadSettings() {
  try {
    return { ...defaults, ...JSON.parse(localStorage.getItem('res-arcana') ?? '{}') };
  } catch {
    return { ...defaults };
  }
}
function saveSettings(settings) {
  try {
    localStorage.setItem('res-arcana', JSON.stringify(settings));
  } catch {
    /* storage may be unavailable; the page works without it */
  }
}
let settings = loadSettings();

async function newGame() {
  try {
    snapshot = await call({ type: 'new_game', players: settings.players, seed: settings.seed, humans: [0], options: [...(settings.draft ? ['draft'] : []), `difficulty=${settings.difficulty}`] });
    viewer = 0;
    error = null;
    saveSettings(settings);
  } catch (e) {
    error = e.message;
  }
  render();
  // Who starts was rolled by dice: show them being thrown.
  if (snapshot?.opening) {
    await playOpening(snapshot.opening, new Map(snapshot.board.players.map((p) => [p.id, p.name])));
  }
}

async function look(seat) {
  try {
    snapshot = await call({ type: 'snapshot', viewer: seat });
    viewer = seat;
    error = null;
  } catch (e) {
    error = e.message;
  }
  render();
}

async function act(action) {
  try {
    snapshot = await call({ type: 'act', viewer, revision: snapshot.revision, action: action.index });
    error = null;
  } catch (e) {
    error = e.message;
    try {
      snapshot = await call({ type: 'snapshot', viewer });
    } catch {
      /* keep the old screen */
    }
  }
  render();
}

/** The turn controls: `set_auto` (with `auto`), `next` (let the next player move) and `undo` (take back this turn). */
async function turn(request) {
  try {
    snapshot = await call({ ...request, viewer });
    error = null;
  } catch (e) {
    error = e.message;
  }
  render();
}

/** Planning: `plan_start`, `plan_undo`, `plan_redo`, `plan_run` (play the list for real) or `plan_stop`. */
async function plan(type) {
  try {
    snapshot = await call({ type, viewer });
    error = null;
  } catch (e) {
    error = e.message;
  }
  render();
}

/** The list of planned moves with undo, redo, "play them" and "leave". */
function planPanel(planned) {
  const done = planned.steps.filter((s) => s.done).length;
  return h(
    'section',
    { class: 'panel plan', id: 'plan' },
    h('h3', null, 'Planned moves'),
    h('p', { class: 'plan-note' }, 'Nothing here is played yet. The board shows what you would have; rivals do not move, and a card you would draw is a guess.'),
    planned.steps.length
      ? h(
          'ol',
          { class: 'plan-steps' },
          planned.steps.map((s) => h('li', { class: s.done ? 'planned' : 'planned undone' }, s.label, s.detail ? h('small', null, ` (${s.detail})`) : null)),
        )
      : h('p', { class: 'muted' }, 'Use your cards as usual: each move is added to this list.'),
    h(
      'div',
      { class: 'plan-buttons' },
      h('button', { id: 'plan-undo', disabled: !planned.can_undo, onclick: () => plan('plan_undo') }, 'Undo'),
      h('button', { id: 'plan-redo', disabled: !planned.can_redo, onclick: () => plan('plan_redo') }, 'Redo'),
      h('button', { id: 'plan-run', class: 'primary', disabled: done === 0, title: 'Plays the moves one after the other, with the rivals answering, and stops at the first one that is no longer possible', onclick: () => plan('plan_run') }, `Play ${done} move${done === 1 ? '' : 's'}`),
      h('button', { id: 'plan-stop', onclick: () => plan('plan_stop') }, 'Leave planning'),
    ),
  );
}

function controls() {
  const players = h(
    'select',
    { id: 'players', onchange: (e) => { settings.players = Number(e.target.value); } },
    [2, 3, 4].map((n) => h('option', { value: n, selected: n === settings.players }, `${n} players`)),
  );
  const levels = ['Novice', 'Easy', 'Medium', 'Hard', 'Expert'];
  const difficulty = h(
    'select',
    { id: 'difficulty', title: 'How well the bots play (applies to the next new game)', onchange: (e) => { settings.difficulty = Number(e.target.value); saveSettings(settings); } },
    levels.map((name, i) => h('option', { value: i + 1, selected: i + 1 === settings.difficulty }, `Bots: ${name}`)),
  );
  const seed = h('input', { id: 'seed', type: 'number', value: settings.seed, onchange: (e) => { settings.seed = Number(e.target.value) || 1; } });
  const seats = snapshot
    ? h(
        'select',
        { id: 'view-as', onchange: (e) => look(Number(e.target.value)) },
        snapshot.board.players.map((p) => h('option', { value: p.id, selected: p.id === viewer }, `${p.name}${p.human ? ' (you)' : ' (bot)'}`)),
      )
    : null;
  return h(
    'div',
    { class: 'controls' },
    players,
    difficulty,
    h('label', null, 'Seed', seed),
    h(
      'label',
      { title: 'Draft variant for the next new game: keep 1 of 4 artifacts and pass the rest (left, then right) instead of a random deck' },
      h('input', { id: 'draft', type: 'checkbox', checked: settings.draft, onchange: (e) => { settings.draft = e.target.checked; saveSettings(settings); } }),
      ' Draft',
    ),
    h(
      'label',
      { title: 'Show the German printed names next to the English ones' },
      h('input', {
        id: 'german',
        type: 'checkbox',
        checked: settings.german,
        onchange: (e) => {
          settings.german = e.target.checked;
          model.showAlt = settings.german;
          saveSettings(settings);
          render();
        },
      }),
      'German names',
    ),
    h(
      'label',
      { title: 'On: the next player moves right after your move. Off: you can undo your move, and press Next when you are done' },
      h('input', {
        id: 'auto-next',
        type: 'checkbox',
        checked: snapshot ? snapshot.auto : true,
        onchange: (e) => turn({ type: 'set_auto', auto: e.target.checked }),
      }),
      'Auto next player',
    ),
    h('button', { id: 'new-game', onclick: newGame }, 'New game'),
    snapshot && !snapshot.winners && snapshot.board.players.find((p) => p.id === viewer)?.human
      ? h(
          'button',
          {
            id: 'plan-toggle',
            class: snapshot.plan ? 'planning' : '',
            title: 'Try moves out and count your essences without playing them; then play the list or drop it',
            onclick: () => plan(snapshot.plan ? 'plan_stop' : 'plan_start'),
          },
          snapshot.plan ? 'Leave planning' : 'Plan moves',
        )
      : null,
    seats ? h('label', null, 'View as', seats) : null,
  );
}

function status() {
  const b = snapshot.board;
  const who = b.active != null ? b.players.find((p) => p.id === b.active)?.name : null;
  return h(
    'div',
    { class: 'status' },
    h('span', null, 'Round ', h('b', null, b.round)),
    h('span', null, 'Phase ', h('b', null, b.phase != null ? model.phaseName(b.phase) : '—')),
    who ? h('span', null, 'Turn ', h('b', null, who)) : null,
  );
}

/** Actions that concern a card nobody can see, or no card at all. */
function looseActions(actions, board) {
  const visible = new Set(board.zones.flatMap((z) => z.cards.map((c) => c.id)));
  return actions.filter((a) => a.kind !== 'decide' && (a.entity == null ? true : !visible.has(a.entity)));
}

function opponent(player, board) {
  const tableau = findZone(board, ZONE.TABLEAU, player.id);
  const hand = findZone(board, ZONE.HAND, player.id);
  const deck = findZone(board, ZONE.DECK, player.id);
  const token = player.counters.find((c) => c.counter === COUNTER.FIRST_TOKEN)?.value > 0;
  return h(
    'section',
    { class: `panel theirs${board.active === player.id ? ' active' : ''}`, 'data-seat': player.id },
    h(
      'div',
      { class: 'panel-head' },
      h('h2', null, player.name, player.human ? '' : ' (bot)'),
      poolView(model, player.pool),
      h('span', { class: 'counts' }, `hand ${hand?.count ?? 0} · deck ${deck?.count ?? 0}${token ? ' · first player token' : ''}${player.passed ? ' · passed' : ''}`),
    ),
    tableau ? zoneView(model, tableau, [], () => {}, { title: 'In play', compact: true }) : null,
  );
}

function mine(me, board, actions, onAct) {
  const tableau = findZone(board, ZONE.TABLEAU, me.id);
  const hand = findZone(board, ZONE.HAND, me.id);
  const candidates = findZone(board, ZONE.CANDIDATES, me.id);
  const draft = findZone(board, ZONE.DRAFT, me.id);
  const token = me.counters.find((c) => c.counter === COUNTER.FIRST_TOKEN)?.value > 0;
  return h(
    'section',
    { class: 'panel me', 'data-seat': me.id },
    h(
      'div',
      { class: 'panel-head' },
      h('h2', null, me.name, me.human ? ' (you)' : ' (bot)'),
      snapshot.awaiting_next ? h('button', { id: 'next-player', class: 'primary', title: 'Let the next player move', onclick: () => turn({ type: 'next' }) }, 'Next') : null,
      snapshot.can_undo ? h('button', { id: 'undo-turn', title: 'Take back what you did this turn', onclick: () => turn({ type: 'undo' }) }, 'Undo') : null,
      poolView(model, me.pool),
      token ? h('span', { class: 'counts' }, 'first player token') : null),
    draft?.cards.length ? zoneView(model, draft, actions, onAct, { title: 'Draft: keep one artifact, pass the rest' }) : null,
    candidates?.cards.length ? zoneView(model, candidates, actions, onAct, { title: 'Mages to choose from' }) : null,
    tableau ? zoneView(model, tableau, actions, onAct, { title: 'In play' }) : null,
    hand ? zoneView(model, hand, actions, onAct, { title: 'Hand' }) : null,
  );
}

function middle(board, actions, onAct) {
  const places = findZone(board, ZONE.PLACES);
  const row = findZone(board, ZONE.MONUMENT_ROW);
  const deck = findZone(board, ZONE.MONUMENT_DECK);
  const items = findZone(board, ZONE.ITEMS);
  const blind = looseActions(actions, board).filter((a) => a.kind === 'buy');
  return h(
    'section',
    { class: 'panel middle' },
    places ? zoneView(model, places, actions, onAct, { title: 'Places of power' }) : null,
    row ? zoneView(model, row, actions, onAct, { title: 'Monuments' }) : null,
    deck
      ? h(
          'div',
          { class: 'blind' },
          h('span', { class: 'counts' }, `Monument deck: ${deck.count} unseen`),
          blind.map((a) => h('button', { class: 'act act-buy', title: a.detail, onclick: () => onAct(a) }, `Claim the top monument (${a.detail || 'free'})`)),
        )
      : null,
    items ? zoneView(model, items, [], () => {}, { title: 'Magic items (swapped when you pass)', compact: true }) : null,
  );
}

function bar(actions, onAct, canAct, deciding, onCards) {
  const others = actions.filter((a) => a.kind === 'pass' || a.kind === 'end_turn');
  // A step done on the cards: each card carries its button, in the order you like; "Done" ends the step once
  // nothing is owed any more (the game does not offer it before).
  const done = onCards?.find((a) => a.skip);
  return h(
    'div',
    { class: 'bar' },
    onCards
      ? [
          h('span', { class: 'turn' }, done ? 'Use the highlighted buttons on your cards in any order, or' : 'Use the highlighted buttons on your cards, in the order you like.'),
          done ? h('button', { class: 'primary', id: 'act-done', onclick: () => onAct(done) }, done.label && done.label !== 'do nothing' ? `Done: ${done.label}` : 'Done') : null,
        ]
      : deciding
      ? h('span', { class: 'turn' }, 'Make your choice in the dialog.')
      : canAct
      ? [h('span', { class: 'turn' }, 'Your move: use a card action above, or'), others.map((a) => h('button', { class: 'primary', id: `act-${a.kind}`, onclick: () => onAct(a) }, a.label))]
      : h('span', { class: 'turn' }, snapshot.winners ? 'The game is over.' : 'Waiting…'),
  );
}

/** Appends nodes to the page, skipping empty slots (append(null) would write the text "null"). */
function put(...nodes) {
  app.append(...nodes.filter(Boolean));
}

function render() {
  hidePopup();
  const scroll = window.scrollY;
  clear(app);
  if (!snapshot) {
    put(h('header', { class: 'top' }, h('h1', null, 'Res Arcana'), controls()), error ? h('div', { class: 'error' }, error) : null);
    return;
  }
  const board = snapshot.board;
  const me = board.players.find((p) => p.id === board.viewer);
  const human = me?.human;
  const actions = human ? snapshot.actions : [];
  const decisions = actions.filter((a) => a.kind === 'decide');
  // Collecting (and any step whose options are abilities of your cards) is played on the cards, not in a dialog.
  const onCards = human && decisions.length > 0 && decidedOnCards(decisions, board);
  const cardActions = onCards ? decisions.filter((a) => a.entity != null) : actions.filter((a) => a.kind !== 'decide');
  const canAct = human && actions.length > 0 && !snapshot.winners;
  const winners = snapshot.winners?.map((w) => board.players.find((p) => p.id === w)?.name).join(' & ');

  put(
    h('header', { class: 'top' }, h('h1', null, 'Res Arcana'), status(), controls()),
    error ? h('div', { class: 'error', role: 'alert' }, error) : null,
    winners ? h('div', { class: 'banner' }, `Game over — ${winners} won`) : null,
    snapshot.plan ? h('div', { class: 'banner planning', role: 'status' }, 'Planning — nothing you do now is played. Use the list on the right to undo, redo, play or drop your plan.') : null,
    snapshot.notice ? h('div', { class: `banner notice${snapshot.alert ? ' alert' : ''}`, role: snapshot.alert ? 'alert' : 'status' }, snapshot.notice) : null,
    h(
      'div',
      { class: 'layout' },
      h(
        'div',
        { class: 'board' },
        // Your area first, so it stays put when the bots' panels change size.
        me ? mine(me, board, cardActions, act) : null,
        middle(board, cardActions, act),
        board.players.filter((p) => p.id !== board.viewer).map((p) => opponent(p, board)),
      ),
      h('aside', { class: 'side' }, snapshot.plan ? planPanel(snapshot.plan) : null, h('section', { class: 'panel' }, h('h3', null, 'Score'), scoreboard(model, snapshot)), h('section', { class: 'panel' }, h('h3', null, 'Log'), logView(model, snapshot.log))),
    ),
    bar(cardActions, act, canAct, decisions.length > 0 && human, onCards ? decisions : null),
    decisions.length && human && !onCards ? decisionDialog(model, decisions, act, { title: 'Your decision' }) : null,
  );
  window.scrollTo(0, scroll);
}

async function main() {
  try {
    const description = await call({ type: 'describe' });
    model = new Model(description);
    model.showAlt = settings.german;
  } catch (e) {
    error = e.message;
    render();
    return;
  }
  await newGame();
}

main();
