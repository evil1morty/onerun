import { state, getProject, getStatus, getCmdStatus } from './state.js';
import { api } from './api.js';
import { $, el, toggle, appendLogLine, flushLogLines, resetLogLines } from './dom.js';
import { runCommand } from './dashboard.js';

const $logPanel = $('log-panel');
const $dash     = $('dashboard');
const $logName  = $('log-project-name');
const $logTabs  = $('log-tabs');
const $logOut   = $('log-output');
const $arrowL   = $('tab-arrow-left');
const $arrowR   = $('tab-arrow-right');
const $logRun    = $('log-run');

// ── Open / Close ───────────────────────────────────

export async function openLogPanel(id) {
  state.activeLogId = id;
  const proj = getProject(id);
  if (!proj) return;

  // Pick default tab: first running command, or first command
  const cmds = state.statuses[id] || {};
  const runningLabel = proj.commands.find(c => cmds[c.label]?.running)?.label;
  state.activeLogTab = runningLabel || proj.commands[0]?.label || null;

  // Subscribe before reading the buffer so nothing printed in between is lost.
  await setLogViewer();

  toggle($logPanel, true);
  $dash.classList.add('blurred');
  updateLogHeader();
  updateLogTabs();

  document.querySelectorAll('.project-row').forEach(r => {
    r.classList.toggle('active', r.dataset.id === id);
  });

  await loadTabLogs();
}

export function closeLogPanel() {
  toggle($logPanel, false);
  $dash.classList.remove('blurred');
  state.activeLogId = null;
  state.activeLogTab = null;
  resetLogLines();
  setLogViewer();
  document.querySelectorAll('.project-row').forEach(r => r.classList.remove('active'));
}

/** Tell the backend which command is on screen — it only streams live log
 *  events for that one. */
export function setLogViewer() {
  return api.setLogViewer(state.activeLogId, state.activeLogTab).catch(() => {});
}

// ── Tab switching ──────────────────────────────────

export async function switchTab(label) {
  if (label === state.activeLogTab) return;
  state.activeLogTab = label;
  await setLogViewer();
  updateLogTabs();
  await loadTabLogs();
}

async function loadTabLogs() {
  resetLogLines();
  $logOut.innerHTML = '';
  if (!state.activeLogId || !state.activeLogTab) {
    showEmptyLog();
    return;
  }
  try {
    const logs = await api.getLogs(state.activeLogId, state.activeLogTab);
    if (logs.length === 0) {
      showEmptyLog();
    } else {
      logs.forEach(l => appendLogLine($logOut, l.text, l.stream));
    }
  } catch (_) {
    showEmptyLog();
  }
}

// ── Log output ─────────────────────────────────────

function showEmptyLog() {
  $logOut.innerHTML = '';
  $logOut.appendChild(el('div', 'log-empty', 'Logs will appear here when you run a command'));
}

export function appendLog(text, stream) {
  // Remove empty state if present
  const empty = $logOut.querySelector('.log-empty');
  if (empty) empty.remove();
  appendLogLine($logOut, text, stream);
}

/** Append a divider line marking that the process finished.
 *  Live-only (not persisted); helps the user tell the command has ended
 *  without needing to read every output line. */
export function appendLogMarker(label) {
  // Land after any lines still queued for this frame.
  flushLogLines();
  const empty = $logOut.querySelector('.log-empty');
  if (empty) empty.remove();

  // Collapse consecutive end markers (e.g. multiple stop signals).
  const last = $logOut.lastElementChild;
  if (last && last.classList.contains('log-marker')) return;

  const time = new Date().toLocaleTimeString([], { hour12: false });
  const div = el('div', 'log-line log-marker');
  div.textContent = `> ${label} finished at ${time}`;
  $logOut.appendChild(div);

  const nearBottom = $logOut.scrollHeight - $logOut.scrollTop - $logOut.clientHeight < 80;
  if (nearBottom) $logOut.scrollTop = $logOut.scrollHeight;
}

// ── Header & tab bar ───────────────────────────────

export function updateLogHeader() {
  const proj = getProject(state.activeLogId);
  $logName.textContent = proj?.name || '';
}

export function updateLogTabs() {
  $logTabs.innerHTML = '';
  const proj = getProject(state.activeLogId);
  if (!proj) {
    toggle($logRun, false);
    return;
  }

  proj.commands.forEach(c => {
    const cs = getCmdStatus(proj.id, c.label);
    const isActive = c.label === state.activeLogTab;

    let cls = 'log-tab';
    if (isActive) cls += ' active';
    if (cs.running) cls += ' running';

    const tab = el('button', cls);

    // Green dot for running commands
    if (cs.running) {
      tab.appendChild(el('span', 'tab-dot'));
    }

    tab.appendChild(document.createTextNode(c.label));
    tab.addEventListener('click', () => switchTab(c.label));
    $logTabs.appendChild(tab);

    // Scroll active tab into view without affecting parent scroll
    if (isActive) {
      requestAnimationFrame(() => {
        const left = tab.offsetLeft - $logTabs.offsetLeft;
        $logTabs.scrollLeft = Math.max(0, left - 20);
      });
    }
  });

  // Start/Stop action button — in the log header with other buttons
  const cs = getCmdStatus(proj.id, state.activeLogTab);
  const cmd = proj.commands.find(c => c.label === state.activeLogTab);

  if (cs.running) {
    $logRun.textContent = 'Stop';
    $logRun.className = 'stop-state';
    $logRun.title = 'Stop command';
    $logRun.onclick = () => api.stopProcess(proj.id, state.activeLogTab);
  } else if (cmd) {
    $logRun.textContent = 'Start';
    $logRun.className = 'run-state';
    $logRun.title = 'Start command';
    $logRun.onclick = () => runCommand(proj.id, cmd.label, cmd.cmd, proj.directory, proj.env);
  } else {
    $logRun.className = 'hidden';
  }

  updateTabArrows();
}

// ── Tab scroll arrows ─────────────────────────────

function updateTabArrows() {
  const overflows = $logTabs.scrollWidth > $logTabs.clientWidth;
  toggle($arrowL, overflows && $logTabs.scrollLeft > 0);
  toggle($arrowR, overflows && $logTabs.scrollLeft + $logTabs.clientWidth < $logTabs.scrollWidth - 1);
}

$arrowL.addEventListener('click', () => {
  $logTabs.scrollBy({ left: -120, behavior: 'smooth' });
  setTimeout(updateTabArrows, 200);
});
$arrowR.addEventListener('click', () => {
  $logTabs.scrollBy({ left: 120, behavior: 'smooth' });
  setTimeout(updateTabArrows, 200);
});
$logTabs.addEventListener('scroll', updateTabArrows);
window.addEventListener('resize', updateTabArrows);

// ── Drag-to-scroll (touchpad / mouse) ────────────
let _dragX = 0, _dragScroll = 0, _dragging = false;

$logTabs.addEventListener('pointerdown', e => {
  if (e.target.closest('.log-tab')) return;
  _dragging = true;
  _dragX = e.clientX;
  _dragScroll = $logTabs.scrollLeft;
  $logTabs.setPointerCapture(e.pointerId);
  $logTabs.style.cursor = 'grabbing';
});
$logTabs.addEventListener('pointermove', e => {
  if (!_dragging) return;
  $logTabs.scrollLeft = _dragScroll - (e.clientX - _dragX);
});
$logTabs.addEventListener('pointerup', () => {
  if (!_dragging) return;
  _dragging = false;
  $logTabs.style.cursor = '';
  updateTabArrows();
});

// ── Button handlers ────────────────────────────────

$dash.addEventListener('click', (e) => {
  if (!$dash.classList.contains('blurred')) return;
  e.preventDefault();
  e.stopImmediatePropagation();
  closeLogPanel();
}, true);
$('log-copy').addEventListener('click', () => {
  flushLogLines(); // include lines queued for this frame
  const lines = $logOut.querySelectorAll('.log-line');
  const text = Array.from(lines).map(l => l.textContent).join('\n');
  navigator.clipboard.writeText(text);
  const copyBtn = $('log-copy');
  copyBtn.textContent = 'Copied!';
  copyBtn.classList.add('copied');
  setTimeout(() => {
    copyBtn.textContent = 'Copy';
    copyBtn.classList.remove('copied');
  }, 1500);
});
$('log-clear').addEventListener('click', () => {
  resetLogLines();
  $logOut.innerHTML = '';
  const clearBtn = $('log-clear');
  clearBtn.textContent = 'Cleared!';
  clearBtn.classList.add('cleared');
  setTimeout(() => {
    clearBtn.textContent = 'Clear';
    clearBtn.classList.remove('cleared');
  }, 1500);
});
$('log-close').addEventListener('click', closeLogPanel);
