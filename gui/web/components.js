// Game-agnostic building blocks: resources, cards, zones, decisions, the log
// and the scoreboard. A game page composes them; nothing here names a game.
import { h, clear } from './dom.js';

const escapeRegExp = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

/** Lookups built from the game's description. */
export class Model {
  constructor(description) {
    this.description = description;
    this.resources = new Map(description.resources.map((r) => [r.id, r]));
    this.cards = new Map(description.cards.map((c) => [c.id, c]));
    this.zones = new Map(description.zones.map((z) => [z.id, z.name]));
    this.counters = new Map(description.counters.map((c) => [c.id, c.name]));
    this.phases = new Map(description.phases.map((p) => [p.id, p.name]));
    this.imageTemplate = description.card_images ?? '';
    /** Show the card's name in its other language (`alt`) next to the main one; off by default. */
    this.showAlt = false;
    this.help = {
      zone: new Map(description.zones.map((z) => [z.id, z.help])),
      counter: new Map(description.counters.map((c) => [c.id, c.help])),
      phase: new Map(description.phases.map((p) => [p.id, p.help])),
      res: new Map(description.resources.map((r) => [r.id, r.help])),
    };
    this.bySymbol = new Map(description.resources.map((r) => [r.symbol, r]));
    const symbols = description.resources.map((r) => escapeRegExp(r.symbol)).sort((a, b) => b.length - a.length);
    this.symbolPattern = symbols.length
      ? new RegExp(`(\\d*)(${symbols.join('|')})(?![A-Za-z])`, 'g')
      : null;
  }
  resource(id) { return this.resources.get(id); }
  card(id) { return this.cards.get(id); }
  /** URL of a card's picture, or null when the game has none. */
  cardImage(id) { return this.imageTemplate ? this.imageTemplate.replace('{id}', String(id)) : null; }
  zoneName(kind) { return this.zones.get(kind) ?? `zone ${kind}`; }
  counterName(id) { return this.counters.get(id) ?? `counter ${id}`; }
  phaseName(id) { return this.phases.get(id) ?? `phase ${id}`; }
}

// ---- Terms: names in marked-up text become underlined, quoted, and explained in a popup ----

const TERM = /\u0001(\w+):(\d+)\u0002([^\u0003]*)\u0003/g;
// Popups form a chain: the first explains a term, and a term inside it opens the next one beside it. The pointer may
// move from the term onto the popup (a short grace delay covers the gap), and the chain closes when it leaves them all.
const popups = [];
let hideTimer = null;

function cancelHide() {
  clearTimeout(hideTimer);
  hideTimer = null;
}

/** Closes every popup now (call when the page is redrawn). */
export function hidePopup() {
  cancelHide();
  while (popups.length) popups.pop().remove();
}

/** Closes the chain shortly, unless the pointer reaches a popup or another term first. */
function leavePopup() {
  cancelHide();
  hideTimer = setTimeout(hidePopup, 200);
}

function closeFrom(depth) {
  while (popups.length > depth) popups.pop().remove();
}

function showPopup(anchor, content, { beside = false } = {}) {
  // An open payment menu must stay clickable: nothing pops up over it.
  if (openPayMenu?.isConnected && !openPayMenu.hidden && !anchor.closest('.popup, .pay-menu')) return;
  cancelHide();
  // A term inside a popup opens the next level; anything else starts a new chain.
  const host = anchor.closest('.popup');
  const depth = host ? popups.indexOf(host) + 1 : 0;
  closeFrom(depth);
  const box = h('div', { class: 'popup', role: 'tooltip' }, content);
  box.addEventListener('mouseenter', cancelHide);
  box.addEventListener('mouseleave', leavePopup);
  document.body.append(box);
  popups.push(box);
  const a = anchor.getBoundingClientRect();
  const b = box.getBoundingClientRect();
  let left = Math.min(Math.max(8, a.left), Math.max(8, window.innerWidth - b.width - 8));
  const below = a.bottom + 2;
  let top = below + b.height > window.innerHeight - 8 ? Math.max(8, a.top - b.height - 2) : below;
  if (beside) {
    // Next to the anchor (a card), so its own buttons and menus stay free: right if there is room, else left.
    const right = a.right + 4;
    const fitsRight = right + b.width <= window.innerWidth - 8;
    const fitsLeft = a.left - 4 - b.width >= 8;
    if (fitsRight || fitsLeft) {
      left = fitsRight ? right : a.left - 4 - b.width;
      top = Math.min(Math.max(8, a.top), Math.max(8, window.innerHeight - b.height - 8));
    }
  }
  box.style.left = `${left}px`;
  box.style.top = `${top}px`;
}

function termContent(model, kind, id, text) {
  if (kind === 'card') {
    return cardView(model, { id: 0, def: id, face_up: true, exhausted: false, pool: [] }, { zoom: 'none' });
  }
  const help = model.help[kind]?.get(id) ?? '';
  const heading = kind === 'res' ? [resChip(model, id, null), ' ', h('b', null, text)] : h('b', null, text);
  return h('div', { class: 'popup-text' }, h('div', { class: 'popup-title' }, heading), help ? h('p', null, help) : null);
}

function term(model, kind, id, text, clickable) {
  const el = h('span', { class: `term term-${kind}` }, `\u201c${text}\u201d`);
  const show = () => showPopup(el, termContent(model, kind, id, text));
  el.addEventListener('mouseenter', show);
  el.addEventListener('mouseleave', leavePopup);
  if (clickable) {
    el.setAttribute('tabindex', '0');
    el.setAttribute('role', 'button');
    el.addEventListener('focus', show);
    el.addEventListener('blur', leavePopup);
    el.addEventListener('click', (event) => {
      // A term inside a button explains itself instead of choosing the option.
      event.stopPropagation();
      event.preventDefault();
      show();
    });
  }
  // A resource name is followed by its colored letter, so it can be told apart at a glance.
  return kind === 'res' ? [el, resDot(model, id)] : el;
}

/**
 * Marked-up text from the game ("move \u0001card:3\u0002Heiler\u0003 ...") as nodes with explained terms.
 * `clickable: false` leaves the terms to hover only, so a click anywhere in a button still presses it.
 */
export function richText(model, markup, { clickable = true } = {}) {
  const out = [];
  let last = 0;
  for (const m of markup.matchAll(TERM)) {
    if (m.index > last) out.push(markup.slice(last, m.index));
    out.push(term(model, m[1], Number(m[2]), m[3], clickable));
    last = m.index + m[0].length;
  }
  if (last < markup.length) out.push(markup.slice(last));
  return out;
}

/** The colored circle holding a resource's letter. */
export function resDot(model, id) {
  const info = model.resource(id);
  return h('b', { class: 'dot', style: { background: info?.color ?? '#888' }, 'aria-hidden': 'true' }, info?.symbol ?? '?');
}

/** An amount of a resource: the number, then the colored circle with its letter. */
export function resChip(model, id, n) {
  const info = model.resource(id);
  return h('span', { class: 'chip', title: `${n ?? ''} ${info?.name ?? ''}`.trim() }, n != null ? h('span', { class: 'chip-n' }, n) : null, resDot(model, id));
}

/** What sits on a card, marked as such ("On card"), so it is not mistaken for a player's pool. */
export function cardPool(model, pool) {
  return h('div', { class: 'card-pool', title: 'Resources on this card' }, h('span', { class: 'card-pool-label' }, 'On card'), pool.map((p) => resChip(model, p.res, p.n)));
}

/** Replaces resource symbols in printed card text ("2L+E") by chips. */
export function symbolize(model, text) {
  const out = [];
  if (!model.symbolPattern) return [text];
  let last = 0;
  for (const m of text.matchAll(model.symbolPattern)) {
    // A symbol inside a word ("Elan", "Life") is not a symbol; avoids a regex lookbehind for older webviews.
    if (m.index > 0 && /[A-Za-z]/.test(text[m.index - 1])) continue;
    if (m.index > last) out.push(text.slice(last, m.index));
    const info = model.bySymbol.get(m[2]);
    out.push(resChip(model, info.id, m[1] === '' ? null : Number(m[1])));
    last = m.index + m[0].length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}

/** A row of chips for a pool ([{res, n}]). */
export function poolView(model, amounts) {
  return h('span', { class: 'pool' }, amounts.length ? amounts.map((a) => resChip(model, a.res, a.n)) : h('i', { class: 'muted' }, 'empty'));
}

const SHORT = { place: 'Place', discard: 'Discard', buy: 'Claim', activate: 'Use', pass: 'Pass', end_turn: 'End turn' };

function shortLabel(model, action) {
  // A decision answered on a card: the button says what that card's ability does.
  if (action.kind === 'decide') return richText(model, action.short || action.markup);
  if (action.kind === 'activate') {
    // The label is "<card>: <effect>"; the button shows the effect, with its terms explained.
    const effect = action.markup.slice(action.markup.indexOf('\u0003') + 1).replace(/^: /, '');
    return richText(model, effect);
  }
  return SHORT[action.kind] ?? action.label;
}

let openPayMenu = null;

/** Several ways of paying for the same action: one button that opens a menu, one entry per way. */
function payMenu(model, group, onAct) {
  const first = group[0];
  const menu = h(
    'div',
    { class: 'pay-menu', role: 'menu', hidden: true },
    group.map((a) =>
      h(
        'button',
        {
          class: 'pay-option',
          role: 'menuitem',
          onclick: (event) => {
            event.stopPropagation();
            menu.hidden = true;
            openPayMenu = null;
            onAct(a);
          },
        },
        richText(model, a.detail_markup || 'free', { clickable: false }),
      ),
    ),
  );
  const toggle = h(
    'button',
    {
      class: `act act-${first.kind}`,
      'aria-haspopup': 'menu',
      'aria-label': `${first.label} \u2014 choose how to pay`,
      onclick: (event) => {
        event.stopPropagation();
        if (openPayMenu && openPayMenu !== menu) openPayMenu.hidden = true;
        menu.hidden = !menu.hidden;
        openPayMenu = menu.hidden ? null : menu;
        if (!menu.hidden) {
          // The card's hover popup would cover the menu: close it, and none opens while the menu is.
          hidePopup();
          document.addEventListener('click', () => {
            menu.hidden = true;
            if (openPayMenu === menu) openPayMenu = null;
          }, { once: true });
        }
      },
    },
    shortLabel(model, first),
    h('small', null, ' \u25be'),
  );
  return h('span', { class: 'act-group' }, toggle, menu);
}

/** Buttons for the actions that concern one card; several ways of paying become a menu. */
export function actionButtons(model, actions, onAct) {
  if (!actions.length) return null;
  const groups = new Map();
  for (const a of actions) {
    const key = `${a.kind}:${a.ability ?? ''}`;
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(a);
  }
  const boxes = [...groups.values()].map((group) => {
    const first = group[0];
    if (group.length === 1) {
      return h(
        'button',
        { class: `act act-${first.kind}`, 'aria-label': `${first.label}${first.detail ? ` — ${first.detail}` : ''}`, onclick: () => onAct(first) },
        shortLabel(model, first),
        first.detail ? h('small', null, ' (', richText(model, first.detail_markup), ')') : null,
      );
    }
    return payMenu(model, group, onAct);
  });
  return h('div', { class: 'card-actions' }, boxes);
}

/** One card. `actions` are the ActionViews that concern it. */
/** A small magnifier on a card; not a <button>, so it can sit inside a clickable option. */
function zoomControl(model, card, info) {
  const open = (event) => {
    event.stopPropagation();
    event.preventDefault();
    showZoom(model, card);
  };
  return h(
    'span',
    {
      class: 'zoom-control',
      role: 'button',
      tabindex: '0',
      title: `Zoom ${info.name}`,
      'aria-label': `Zoom ${info.name}`,
      onclick: open,
      onkeydown: (event) => {
        if (event.key === 'Enter' || event.key === ' ') open(event);
      },
    },
    '\u2922',
  );
}

/** A card shown large, with its printed text, over the page. Click, Esc or the cross closes it. */
export function showZoom(model, card) {
  const info = card.def != null ? model.card(card.def) : null;
  if (!info) return;
  hidePopup();
  const previous = document.activeElement;
  const url = model.cardImage(card.def);
  const onKey = (event) => {
    if (event.key === 'Escape') close();
  };
  const close = () => {
    overlay.remove();
    document.removeEventListener('keydown', onKey);
    previous?.focus?.();
  };
  const overlay = h(
    'div',
    { class: 'zoom', role: 'dialog', 'aria-modal': 'true', 'aria-label': info.name, tabindex: '-1', onclick: close },
    h(
      'div',
      { class: 'zoom-box', onclick: (event) => event.stopPropagation() },
      h(
        'div',
        { class: 'zoom-head' },
        h('h2', null, info.name),
        model.showAlt && info.alt ? h('span', { class: 'muted' }, info.alt) : null,
        h('button', { class: 'zoom-close', 'aria-label': 'Close', onclick: close }, '\u00d7'),
      ),
      url ? h('img', { class: 'zoom-art', src: url, alt: info.name, onerror: (event) => event.target.remove() }) : null,
      h('div', { class: 'card-tags' }, info.tags.join(' \u00b7 '), info.cost ? [' \u00b7 cost ', symbolize(model, info.cost)] : null, info.points ? ` \u00b7 ${info.points} VP` : null),
      h('div', { class: 'zoom-lines' }, info.lines.map((line) => h('div', { class: 'card-line' }, symbolize(model, line)))),
      info.powers?.length ? h('div', { class: 'zoom-powers' }, info.powers.map((line) => h('p', { class: 'power-line' }, richText(model, line, { clickable: false })))) : null,
      card.pool?.length ? cardPool(model, card.pool) : null,
    ),
  );
  document.body.append(overlay);
  document.addEventListener('keydown', onKey);
  overlay.focus();
}

/**
 * Zooming: `zoom: 'card'` (default) lets a click anywhere on the card (not on its buttons) open the large view, plus a
 * small zoom control; `'button'` keeps only the control (for cards inside a clickable option); `'none'` has neither.
 */
export function cardView(model, card, { actions = [], onAct = () => {}, compact = false, zoom = 'card' } = {}) {
  const info = card.def != null ? model.card(card.def) : null;
  const url = info ? model.cardImage(card.def) : null;
  const el = h(
    'div',
    {
      class: `card${card.exhausted ? ' turned' : ''}${info ? '' : ' back'}${compact ? ' compact' : ''}${info && zoom === 'card' ? ' zoomable' : ''}`,
      'data-card': card.id,
      onclick: info && zoom === 'card'
        ? (event) => {
            // Buttons, menus and explained terms keep their own meaning.
            if (!event.target.closest('button, select, .term, .zoom-control')) showZoom(model, card);
          }
        : null,
    },
    url
      ? h('img', {
          class: 'card-art',
          src: url,
          alt: info.name,
          loading: 'lazy',
          // The picture replaces the printed text once it loads; without a file the card stays text.
          onload: (event) => event.target.closest('.card')?.classList.add('has-art'),
          onerror: (event) => event.target.remove(),
        })
      : null,
    h(
      'div',
      { class: 'card-head' },
      h('span', { class: 'card-name' }, info ? info.name : 'Hidden card'),
      info?.cost ? h('span', { class: 'card-cost' }, symbolize(model, info.cost)) : null,
      info && zoom !== 'none' ? zoomControl(model, card, info) : null,
    ),
    info ? h('div', { class: 'card-tags' }, info.tags.join(' · '), info.points ? h('span', { class: 'card-vp' }, ` ${info.points} VP`) : null) : null,
    info && !compact ? h('div', { class: 'card-lines' }, info.lines.map((l) => h('div', { class: 'card-line' }, symbolize(model, l)))) : null,
    card.pool.length ? cardPool(model, card.pool) : null,
    actionButtons(model, actions, onAct),
  );
  // Hovering a card explains, in English, what its abilities do.
  if (info?.powers?.length && zoom !== 'none') {
    el.addEventListener('mouseenter', () => showPopup(el, powersContent(model, info), { beside: true }));
    el.addEventListener('mouseleave', leavePopup);
  }
  return el;
}

/** The English description of what a card does (the hover popup). */
function powersContent(model, info) {
  return h(
    'div',
    { class: 'popup-text powers' },
    h('div', { class: 'popup-title' }, h('b', null, info.name), model.showAlt && info.alt ? h('span', { class: 'muted' }, ` \u00b7 ${info.alt}`) : null),
    info.powers.map((line) => h('p', { class: 'power-line' }, richText(model, line, { clickable: false }))),
  );
}

/** The zone with this role and owner (`owner` null = shared). */
export function findZone(board, kind, owner = null) {
  return board.zones.find((z) => z.kind === kind && (owner == null ? z.owner == null : z.owner?.includes(owner)));
}

/**
 * Whether a pending decision can be answered on the cards themselves: every option uses an ability of a card the
 * viewer sees on the board, except the one that does nothing (which becomes a "Done" button). This is how a step with
 * several things to do in any order is played (collecting in Res Arcana): each card carries its own button.
 */
export function decidedOnCards(decisions, board) {
  if (!decisions.some((a) => a.entity != null)) return false;
  const visible = new Set(board.zones.flatMap((z) => z.cards.map((c) => c.id)));
  return decisions.every((a) => a.skip || (a.entity != null && a.short && visible.has(a.entity)));
}

/** A titled row of the cards of a zone, with the actions that belong to each card. */
export function zoneView(model, zone, actions, onAct, { title, compact = false } = {}) {
  const byCard = new Map();
  for (const a of actions) {
    if (a.entity == null) continue;
    if (!byCard.has(a.entity)) byCard.set(a.entity, []);
    byCard.get(a.entity).push(a);
  }
  const heading = `${title ?? model.zoneName(zone.kind)} (${zone.count})`;
  const hidden = zone.count > zone.cards.length;
  return h(
    'section',
    { class: 'zone', 'data-zone': zone.kind },
    h('h3', null, heading),
    h(
      'div',
      { class: 'cards' },
      zone.cards.map((c) => cardView(model, c, { actions: byCard.get(c.id) ?? [], onAct, compact })),
      hidden ? h('div', { class: 'card back' }, `${zone.count - zone.cards.length} hidden`) : null,
    ),
  );
}

/**
 * A decision that is only a mix of resources (every option gives, or every option takes): pick the amount of each kind
 * with + and -, with a counter of how many are still to give or obtain. Red is spent, green is gained. The confirm
 * button plays the option that matches the mix.
 */
export function mixDialog(model, actions, onAct, { title } = {}) {
  const gain = actions[0].mix.direction === 'gain';
  const sign = gain ? '+' : '\u2212';
  const kinds = [...new Set(actions.flatMap((a) => a.mix.amounts.map((m) => m.res)))].sort((x, y) => x - y);
  const total = (a) => a.mix.amounts.reduce((sum, m) => sum + m.n, 0);
  const need = Math.max(...actions.map(total));
  const most = new Map(kinds.map((k) => [k, Math.max(...actions.map((a) => a.mix.amounts.filter((m) => m.res === k).reduce((s, m) => s + m.n, 0)))]));
  const key = (counts) => kinds.map((k) => counts.get(k) ?? 0).join(',');
  const options = new Map(actions.map((a) => [key(new Map(kinds.map((k) => [k, a.mix.amounts.filter((m) => m.res === k).reduce((s, m) => s + m.n, 0)]))), a]));
  const picked = new Map(kinds.map((k) => [k, 0]));
  const sum = () => [...picked.values()].reduce((a, b) => a + b, 0);

  const counter = h('div', { class: `mix-counter ${gain ? 'gain' : 'spend'}`, role: 'status' });
  const confirm = h('button', { class: 'mix-confirm primary' }, 'Confirm');
  const rows = kinds.map((k) => {
    const value = h('span', { class: 'mix-value' });
    const minus = h('button', { class: 'mix-step', 'aria-label': `One fewer ${model.resource(k)?.name ?? ''}`, onclick: () => { picked.set(k, picked.get(k) - 1); update(); } }, '\u2212');
    const plus = h('button', { class: 'mix-step', 'aria-label': `One more ${model.resource(k)?.name ?? ''}`, onclick: () => { picked.set(k, picked.get(k) + 1); update(); } }, '+');
    const row = h('div', { class: `mix-row ${gain ? 'gain' : 'spend'}` }, resChip(model, k, null), h('span', { class: 'mix-name' }, model.resource(k)?.name ?? ''), minus, value, plus);
    return { k, value, minus, plus, row };
  });
  function update() {
    const left = need - sum();
    const match = options.get(key(picked));
    counter.textContent = left > 0 ? `${gain ? 'Obtain' : 'Give'} ${left} more` : left === 0 && match ? `${gain ? 'Obtaining' : 'Giving'} ${need} \u2014 ready` : `${need} chosen`;
    for (const r of rows) {
      const n = picked.get(r.k);
      r.value.textContent = n > 0 ? `${sign}${n}` : '0';
      r.value.classList.toggle('active', n > 0);
      r.minus.disabled = n === 0;
      r.plus.disabled = left <= 0 || n >= most.get(r.k);
    }
    confirm.disabled = !match;
  }
  confirm.addEventListener('click', () => {
    const match = options.get(key(picked));
    if (match) onAct(match);
  });
  update();
  return h(
    'div',
    { class: 'modal', role: 'dialog', 'aria-modal': 'true', 'aria-label': title ?? 'Choose resources' },
    h('div', { class: 'modal-box mix-box' }, h('h2', null, title ?? 'Choose resources'), counter, h('div', { class: 'mix-rows' }, rows.map((r) => r.row)), confirm),
  );
}

/**
 * A decision where every option is one number of a single kind of resource (identical essences): one button per kind,
 * so nothing else can be picked. Mixes of kinds use `mixDialog` instead.
 */
export function kindDialog(model, actions, onAct, { title } = {}) {
  const gain = actions[0].mix.direction === 'gain';
  const sign = gain ? '+' : '\u2212';
  const button = (a) => {
    const total = a.mix.amounts.reduce((sum, m) => sum + m.n, 0);
    const k = a.mix.amounts[0].res;
    return h(
      'button',
      { class: `option kind-option ${gain ? 'gain' : 'spend'}`, 'data-option': a.index, 'aria-label': a.label, onclick: () => onAct(a) },
      resChip(model, k, null),
      h('span', { class: 'option-label' }, `${sign}${total} ${model.resource(k)?.name ?? ''}`),
    );
  };
  return h(
    'div',
    { class: 'modal', role: 'dialog', 'aria-modal': 'true', 'aria-label': title ?? 'Choose resources' },
    h('div', { class: 'modal-box mix-box' }, h('h2', null, title ?? 'Choose resources'), h('div', { class: 'options' }, actions.map(button))),
  );
}

/**
 * The options of a pending decision, as a modal. An option that is about cards
 * shows them as cards (what the viewer may see of them), with the words below.
 */
export function decisionDialog(model, actions, onAct, { title = 'Your decision' } = {}) {
  const direction = actions[0]?.mix?.direction;
  const oneKind = (a) => a.mix && a.mix.direction === direction && new Set(a.mix.amounts.map((m) => m.res)).size === 1;
  if (actions.length > 1 && actions.every(oneKind)) {
    return kindDialog(model, actions, onAct, { title: direction === 'gain' ? 'Choose what you obtain' : 'Choose what you give' });
  }
  if (actions.length > 1 && actions.every((a) => a.mix && a.mix.direction === actions[0].mix.direction)) {
    return mixDialog(model, actions, onAct, { title: actions[0].mix.direction === 'gain' ? 'Choose what you obtain' : 'Choose what you give' });
  }
  // Three or more options that are all cards (a magic item to pick, a card to straighten) sit on a shelf: compact
  // tiles in up to 4 columns, the words only in the tooltip. One or two options keep their larger cards and text.
  const shelf = actions.length >= 3 && actions.every((a) => a.cards?.length);
  const option = (a) =>
    h(
      'button',
      {
        class: `option${a.cards?.length ? ' with-cards' : ''}${shelf ? ' tile' : ''}`,
        'data-option': a.index,
        'aria-label': a.label,
        onclick: () => onAct(a),
      },
      a.cards?.length ? h('span', { class: 'option-cards' }, a.cards.map((c) => cardView(model, c, { compact: shelf, zoom: 'button' }))) : null,
      shelf ? null : h('span', { class: 'option-label' }, richText(model, a.markup)),
    );
  return h(
    'div',
    { class: 'modal', role: 'dialog', 'aria-modal': 'true', 'aria-label': title },
    h(
      'div',
      { class: `modal-box${shelf ? ' wide' : ''}` },
      h('h2', null, title),
      h('div', { class: `options${shelf ? ' shelf' : ''}`, style: shelf ? { '--cols': Math.min(4, actions.length) } : null }, actions.map(option)),
    ),
  );
}

/** The game log, newest at the bottom. */
export function logView(model, lines) {
  const box = h('div', { class: 'log' }, lines.map((l) => h('div', { class: `log-line${l.player != null ? ` p${l.player}` : ''}` }, richText(model, l.markup))));
  queueMicrotask(() => { box.scrollTop = box.scrollHeight; });
  return box;
}

/** Seats with their scores, pools and counters. */
export function scoreboard(model, snapshot) {
  const { board, scores } = snapshot;
  return h(
    'table',
    { class: 'scores' },
    board.players.map((p) =>
      h(
        'tr',
        { class: `seat p${p.id}${board.active === p.id ? ' active' : ''}${p.passed ? ' passed' : ''}` },
        h('th', null, p.name, p.human ? ' (you)' : ' (bot)', p.passed ? h('small', null, ' · passed') : null),
        h('td', null, scores.filter((s) => s.player === p.id).map((s) => h('span', { class: 'score' }, `${s.label}: `, h('b', null, s.value)))),
        h('td', null, poolView(model, p.pool)),
      ),
    ),
  );
}

// ---- Dice: who starts ----

// Which pips each face shows, as cells of a 3x3 grid (0..8, row by row).
const PIPS = { 1: [4], 2: [0, 8], 3: [0, 4, 8], 4: [0, 2, 6, 8], 5: [0, 2, 4, 6, 8], 6: [0, 2, 3, 5, 6, 8] };
// The turn (x, y in degrees) that brings each face to the front.
const FACE_TURN = { 1: [0, 0], 2: [0, -90], 3: [-90, 0], 4: [90, 0], 5: [0, 90], 6: [0, 180] };
// Where each face sits on the cube (the opposite faces add up to 7).
const FACE_PLACE = { 1: '', 2: 'rotateY(90deg)', 3: 'rotateX(90deg)', 4: 'rotateX(-90deg)', 5: 'rotateY(-90deg)', 6: 'rotateY(180deg)' };

function dieFace(value) {
  const cells = PIPS[value];
  return h(
    'div',
    { class: 'die-face', style: { transform: `${FACE_PLACE[value]} translateZ(var(--die-half))` } },
    Array.from({ length: 9 }, (_, i) => h('span', { class: cells.includes(i) ? 'pip' : 'pip off' })),
  );
}

function dieCube() {
  return h('div', { class: 'die-cube' }, [1, 2, 3, 4, 5, 6].map(dieFace));
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/**
 * Plays the dice that decided who starts: every contender throws a die, the
 * highest roll wins, and tied players throw again. `opening` is the snapshot's
 * `opening`, `names` maps a seat to its name. Resolves when it is over (or
 * the viewer skipped it).
 */
export async function playOpening(opening, names) {
  const reduced = window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
  const speed = reduced ? 0.25 : 1;
  let skipped = false;
  const title = h('h3', { class: 'dice-title' }, 'Rolling for the first player…');
  const tray = h('div', { class: 'dice-tray' });
  const note = h('p', { class: 'dice-note' }, ' ');
  const skip = h('button', { class: 'dice-skip', onclick: () => { skipped = true; } }, 'Skip');
  const overlay = h('div', { class: 'modal dice-overlay', role: 'status' }, h('div', { class: 'modal-box dice-box' }, title, tray, note, skip));
  document.body.append(overlay);
  const wait = async (ms) => {
    const end = Date.now() + ms * speed;
    while (!skipped && Date.now() < end) await sleep(40);
  };
  try {
    for (const [index, round] of opening.rounds.entries()) {
      if (skipped) break;
      clear(tray);
      title.textContent = index === 0 ? 'Rolling for the first player…' : 'A tie! Rolling again…';
      note.textContent = ' ';
      const slots = round.map((roll) => {
        const cube = dieCube();
        const label = h('div', { class: 'die-name' }, names.get(roll.player) ?? `Player ${roll.player + 1}`);
        const slot = h('div', { class: 'die-slot' }, h('div', { class: 'die-scene' }, cube), label);
        tray.append(slot);
        return { roll, cube, slot };
      });
      // Every die is thrown from above with a tumble, one after the other.
      const flights = slots.map(({ roll, cube, slot }, i) => {
        const [x, y] = FACE_TURN[roll.value];
        const spin = 540 + 180 * (i % 2);
        const rest = `rotateX(${x}deg) rotateY(${y}deg) rotateZ(0deg)`;
        cube.style.transform = rest;
        const duration = (1300 + i * 260) * speed;
        const fall = slot.querySelector('.die-scene').animate(
          [
            { transform: 'translateY(-220px) scale(1.25)', opacity: 0, offset: 0 },
            { transform: 'translateY(0) scale(1)', opacity: 1, offset: 0.55, easing: 'ease-in' },
            { transform: 'translateY(-46px) scale(1)', offset: 0.75, easing: 'ease-out' },
            { transform: 'translateY(0) scale(1)', offset: 0.9, easing: 'ease-in' },
            { transform: 'translateY(-10px) scale(1)', offset: 0.96, easing: 'ease-out' },
            { transform: 'translateY(0) scale(1)', offset: 1 },
          ],
          { duration, easing: 'linear', fill: 'both', delay: i * 120 * speed },
        );
        const tumble = cube.animate(
          [{ transform: `rotateX(${x + 720}deg) rotateY(${y - spin}deg) rotateZ(${360 + 90 * i}deg)` }, { transform: rest }],
          { duration, easing: 'cubic-bezier(.15,.6,.3,1)', fill: 'both', delay: i * 120 * speed },
        );
        return Promise.all([fall.finished, tumble.finished]).catch(() => {});
      });
      await Promise.race([Promise.all(flights), (async () => { while (!skipped) await sleep(40); })()]);
      if (skipped) {
        // Show where the dice landed without the flight.
        for (const { slot, cube } of slots) {
          for (const a of [...slot.querySelector('.die-scene').getAnimations(), ...cube.getAnimations()]) a.finish();
        }
      }
      const best = Math.max(...round.map((r) => r.value));
      const leaders = slots.filter((s) => s.roll.value === best);
      for (const s of slots) s.slot.classList.add(s.roll.value === best ? 'top' : 'out');
      if (index === opening.rounds.length - 1) {
        const winner = names.get(opening.first) ?? `Player ${opening.first + 1}`;
        title.textContent = `${winner} starts!`;
        note.textContent = `${winner} rolled the highest (${best}).`;
      } else {
        note.textContent = `${leaders.map((s) => names.get(s.roll.player)).join(' and ')} tie with ${best}.`;
      }
      await wait(index === opening.rounds.length - 1 ? 2000 : 1500);
    }
  } finally {
    overlay.remove();
  }
}
