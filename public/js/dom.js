import { ansiToHtml } from './ansi.js';

// ── DOM helpers ────────────────────────────────────

/** Shorthand for getElementById */
export const $ = (id) => document.getElementById(id);

/** Create an element with optional class and text */
export function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text != null) node.textContent = text;
  return node;
}

/** Create a button with class, label, and click handler */
export function btn(className, label, onClick) {
  const b = el('button', className, label);
  if (onClick) b.addEventListener('click', onClick);
  return b;
}

/** Toggle the 'hidden' class */
export function toggle(node, visible) {
  node.classList.toggle('hidden', !visible);
}

// ── Log output ─────────────────────────────────────
// Lines are queued and flushed once per animation frame. Appending each line
// on arrival meant one forced layout per line (the scroll-position read), so
// a burst of output from a dev server would lock up the UI.

const MAX_LOG_NODES = 2000;

let _logQueue = [];
let _logContainer = null;
let _logRaf = 0;

function makeLogLine(text, stream) {
  const div = document.createElement('div');
  div.className = 'log-line ' + (stream || 'stdout');
  if (text.includes('\x1b')) {
    div.innerHTML = ansiToHtml(text);
  } else {
    div.textContent = text;
  }
  return div;
}

/** Queue a log line for the next frame. */
export function appendLogLine(container, text, stream) {
  if (_logContainer && _logContainer !== container) flushLogLines();
  _logContainer = container;
  _logQueue.push([text, stream]);

  // Frames stop firing while the window is hidden in the tray, so the queue
  // has to be self-limiting. Anything past MAX_LOG_NODES would be trimmed off
  // the top on flush anyway; drop it in batches to keep this O(1) amortized.
  if (_logQueue.length > MAX_LOG_NODES * 2) {
    _logQueue.splice(0, _logQueue.length - MAX_LOG_NODES);
  }

  if (!_logRaf) _logRaf = requestAnimationFrame(flushLogLines);
}

/** Write every queued line in one pass. Safe to call at any time. */
export function flushLogLines() {
  if (_logRaf) { cancelAnimationFrame(_logRaf); _logRaf = 0; }
  const container = _logContainer;
  if (!container || _logQueue.length === 0) { _logQueue.length = 0; return; }

  // Measure before appending — afterwards we are never "near the bottom".
  const nearBottom = container.scrollHeight - container.scrollTop - container.clientHeight < 80;

  const frag = document.createDocumentFragment();
  for (const [text, stream] of _logQueue) frag.appendChild(makeLogLine(text, stream));
  _logQueue.length = 0;
  container.appendChild(frag);

  let over = container.children.length - MAX_LOG_NODES;
  while (over-- > 0) container.removeChild(container.firstChild);

  if (nearBottom) container.scrollTop = container.scrollHeight;
}

/** Drop queued lines — call before clearing the log element, so pending
 *  output from the previous tab can't land in the new one. */
export function resetLogLines() {
  if (_logRaf) { cancelAnimationFrame(_logRaf); _logRaf = 0; }
  _logQueue.length = 0;
  _logContainer = null;
}

/** Close an overlay when clicking the backdrop */
export function closeOnBackdrop(overlay, closeFn) {
  overlay.addEventListener('click', e => {
    if (e.target === overlay) closeFn();
  });
}

// Curated, high-contrast palette (readable on both dark and light themes).
const TAG_PALETTE = [
  '#58a6ff', // blue
  '#3fb950', // green
  '#f78166', // orange
  '#bc8cff', // purple
  '#ec6cb9', // pink
  '#39c5cf', // teal
  '#e3b341', // yellow
  '#ff7b72', // red
  '#a371f7', // violet
  '#56d364', // light green
  '#ffa657', // amber
  '#79c0ff', // sky
  '#ff9e64', // peach
  '#9ece6a', // lime
  '#f7768e', // rose
  '#7dcfff', // cyan
];

let _tagColorMap = new Map();

/** Rebuild the deterministic tag→color map so each known tag gets a
 *  distinct palette slot (assigned alphabetically). Call whenever the
 *  set of project tags changes. */
export function rebuildTagColors(tags) {
  const sorted = [...new Set(tags)].sort();
  _tagColorMap = new Map();
  sorted.forEach((tag, i) => {
    _tagColorMap.set(tag, TAG_PALETTE[i % TAG_PALETTE.length]);
  });
}

/** Color for a tag — uses the rebuilt map, falling back to a stable hash. */
export function tagColor(tag) {
  const cached = _tagColorMap.get(tag);
  if (cached) return cached;
  let h = 0;
  for (let i = 0; i < tag.length; i++) {
    h = (h * 31 + tag.charCodeAt(i)) >>> 0;
  }
  return TAG_PALETTE[h % TAG_PALETTE.length];
}
