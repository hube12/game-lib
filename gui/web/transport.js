// How the page talks to the game: a Tauri command inside the desktop app, an
// HTTP POST to the dev server in a browser. Both carry the same JSON.

/** Sends a request object and resolves to the response object (throws on errors). */
export async function call(request) {
  const tauri = window.__TAURI__;
  let text;
  if (tauri?.core?.invoke) {
    text = await tauri.core.invoke('api', { request: JSON.stringify(request) });
  } else {
    const reply = await fetch('/api', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(request),
    });
    text = await reply.text();
  }
  const response = JSON.parse(text);
  if (response.type === 'error') throw new Error(response.message);
  return response;
}
