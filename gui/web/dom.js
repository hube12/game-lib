// Tiny DOM helpers shared by every game page.

/** Creates an element: h('div', {class: 'x', onclick: fn}, child, [children]...) */
export function h(tag, props, ...children) {
  const el = document.createElement(tag);
  for (const [key, value] of Object.entries(props ?? {})) {
    if (value === false || value == null) continue;
    if (key === 'class') el.className = value;
    else if (key === 'style' && typeof value === 'object') {
      // Custom properties ("--cols") need setProperty; plain assignment ignores them.
      for (const [name, v] of Object.entries(value)) {
        if (name.startsWith('--')) el.style.setProperty(name, String(v));
        else el.style[name] = v;
      }
    }
    else if (key.startsWith('on')) el.addEventListener(key.slice(2).toLowerCase(), value);
    else if (value === true) el.setAttribute(key, '');
    else el.setAttribute(key, value);
  }
  for (const child of children.flat(Infinity)) {
    if (child == null || child === false) continue;
    el.append(child.nodeType ? child : document.createTextNode(String(child)));
  }
  return el;
}

/** Removes everything inside an element and returns it. */
export function clear(el) {
  el.replaceChildren();
  return el;
}
