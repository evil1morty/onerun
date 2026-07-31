import { state } from './state.js';
import { api } from './api.js';
import { $ } from './dom.js';
import { render } from './dashboard.js';

const $overlay    = $('settings-overlay');
const $claude     = $('set-claude');
const $claudeMode = $('set-claude-mode');
const $editor     = $('set-editor');
const $projectsDir = $('set-projects-dir');
const $autostart  = $('set-autostart');
const $savedPill  = $('settings-saved-pill');

// ── Open / Close ───────────────────────────────────

export async function openSettings() {
  $claude.value     = state.settings.claude_command;
  $claudeMode.value = state.settings.claude_mode || 'window';
  $editor.value     = state.settings.editor_command;
  $projectsDir.value = state.settings.projects_dir || '';
  if (!$projectsDir.placeholder) {
    try { $projectsDir.placeholder = await api.getDefaultProjectsDir(); } catch (_) {}
  }
  try { $autostart.checked = await api.getAutostart(); } catch (_) { $autostart.checked = false; }
  $overlay.classList.remove('hidden');
}

function closeSettings() {
  // Flush any pending debounced save so the user doesn't lose a final keystroke.
  if (_saveTimer) {
    clearTimeout(_saveTimer);
    _saveTimer = null;
    persist();
  }
  $overlay.classList.add('hidden');
}

// ── Apply theme (header toggle + system listener) ──

export function applyTheme(theme) {
  let effective;
  if (theme === 'system') {
    effective = window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
  } else {
    effective = theme;
  }
  document.body.setAttribute('data-theme', effective);
  // Icon reflects the SETTING (light / dark / system), not the resolved theme.
  const sun    = $('theme-icon-sun');
  const moon   = $('theme-icon-moon');
  const system = $('theme-icon-system');
  if (sun && moon && system) {
    sun.classList.toggle('hidden',    theme !== 'light');
    moon.classList.toggle('hidden',   theme !== 'dark');
    system.classList.toggle('hidden', theme !== 'system');
  }
  const btn = $('btn-theme');
  if (btn) {
    const label = theme === 'light' ? 'Light' : theme === 'dark' ? 'Dark' : 'System';
    btn.title = `Theme: ${label} (click to cycle)`;
  }
}

// ── Auto-save ──────────────────────────────────────

let _saveTimer = null;
let _pillTimer = null;

function flashSaved() {
  if (!$savedPill) return;
  $savedPill.classList.add('show');
  clearTimeout(_pillTimer);
  _pillTimer = setTimeout(() => $savedPill.classList.remove('show'), 900);
}

async function persist() {
  state.settings.claude_command = $claude.value.trim() || 'claude';
  state.settings.claude_mode    = $claudeMode.value;
  state.settings.editor_command = $editor.value.trim() || 'code';
  state.settings.projects_dir   = $projectsDir.value.trim();
  state.settings.autostart      = $autostart.checked;
  try {
    await api.saveSettings(state.settings);
    try { await api.setAutostart($autostart.checked); } catch (_) {}
    flashSaved();
  } catch (_) {}
}

function scheduleSave() {
  clearTimeout(_saveTimer);
  _saveTimer = setTimeout(persist, 350);
}

[$claude, $editor, $projectsDir].forEach(el => el.addEventListener('input', scheduleSave));
$claudeMode.addEventListener('change', persist);
$autostart.addEventListener('change', persist);

// ── Events ─────────────────────────────────────────

window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => {
  if (state.settings.theme === 'system') applyTheme('system');
});

$('about-github').addEventListener('click', (e) => {
  e.preventDefault();
  api.openInBrowser('https://github.com/evil1morty/onerun');
});

$('btn-settings').addEventListener('click', openSettings);

$('btn-toggle-tags').addEventListener('click', async () => {
  const next = state.settings.tags_visible === false ? true : false;
  state.settings.tags_visible = next;
  render();
  try { await api.saveSettings(state.settings); } catch (_) {}
});

$('btn-theme').addEventListener('click', async () => {
  // Cycle through light → dark → system → light…
  const order = ['light', 'dark', 'system'];
  const cur = state.settings.theme || 'system';
  const next = order[(order.indexOf(cur) + 1) % order.length];
  state.settings.theme = next;
  applyTheme(next);
  try { await api.saveSettings(state.settings); } catch (_) {}
});

$('settings-cancel').addEventListener('click', closeSettings);
