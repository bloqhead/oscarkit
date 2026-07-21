// Classic AIM messages carry rich text as raw HTML in the ICBM message body
// — oscar-rs passes that text through completely transparently in both
// directions (it's just bytes to the wire format), so formatting is a
// frontend-only concern: wrap on send (see ImScreen.vue), sanitize on
// render (here).
//
// This is a strict allow-list sanitizer, not a strip-the-bad-stuff one:
// only b/i/u/font(color) survive, every other tag is unwrapped (its safe
// children kept, the tag itself dropped), and script/style are dropped
// entirely including their content. Every output element is freshly built
// via createElement/setAttribute against a fixed allow-list — untrusted
// attributes or markup are never copied through as strings, so there's no
// string-concatenation path for a malicious payload to ride along on.
const ALLOWED_TAGS = new Set(['b', 'i', 'u', 'font']);
const HEX_COLOR = /^#[0-9a-fA-F]{6}$/;

function sanitizeInto(node: ChildNode, target: Node): void {
  if (node.nodeType === Node.TEXT_NODE) {
    target.appendChild(document.createTextNode(node.textContent ?? ''));
    return;
  }
  if (node.nodeType !== Node.ELEMENT_NODE) return;

  const el = node as Element;
  const tag = el.tagName.toLowerCase();

  if (tag === 'script' || tag === 'style') return;

  if (ALLOWED_TAGS.has(tag)) {
    const clean = document.createElement(tag);
    if (tag === 'font') {
      const color = el.getAttribute('color');
      if (color && HEX_COLOR.test(color)) clean.setAttribute('color', color);
    }
    for (const child of Array.from(node.childNodes)) sanitizeInto(child, clean);
    target.appendChild(clean);
    return;
  }

  // Disallowed tag: unwrap it — keep its sanitized children, drop the tag.
  for (const child of Array.from(node.childNodes)) sanitizeInto(child, target);
}

export function sanitizeFormattedMessage(html: string): string {
  const parsed = new DOMParser().parseFromString(`<div>${html}</div>`, 'text/html');
  const root = parsed.body.firstElementChild;
  if (!root) return '';

  const container = document.createElement('div');
  for (const child of Array.from(root.childNodes)) sanitizeInto(child, container);
  return container.innerHTML;
}

// Escapes a plain-text message before formatting tags get wrapped around
// it, so literal <, >, & typed by the user can't be mistaken for markup by
// us or by whoever receives it.
export function escapeMessageText(text: string): string {
  const div = document.createElement('div');
  div.textContent = text;
  return div.innerHTML;
}
