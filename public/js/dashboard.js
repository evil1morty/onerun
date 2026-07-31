import { state, getStatus, getCmdStatus, checkProjectPaths } from './state.js';
import { api } from './api.js';
import { $, el, btn, toggle, appendLogLine, resetLogLines, tagColor, rebuildTagColors } from './dom.js';
import { openContextMenu, showConfirm } from './context-menu.js';
import { openLogPanel, closeLogPanel } from './logs.js';
import { openDialog } from './dialog.js';
import { toast } from './toast.js';

const HIDDEN_TAG = 'hidden';

const $list        = $('project-list');
const $empty       = $('empty');
const $table       = $('project-table');
const $search      = $('search');
const $searchClear = $('search-clear');
const $tagBar      = $('tag-bar');
const $tagBarList  = $('tag-bar-list');
const $tagClearBtn = $('tag-clear-btn');

// ── Render ─────────────────────────────────────────

export function render() {
  // Tags from non-hidden projects feed the color palette & tag bar.
  const visibleProjects = state.projects.filter(p => !(p.tags || []).includes(HIDDEN_TAG));
  rebuildTagColors(visibleProjects.flatMap(p => p.tags || []));
  applyTagsVisibility();
  renderTagBar();

  if (state.projects.length === 0) {
    toggle($table, false);
    toggle($empty, true);
    return;
  }
  toggle($table, true);
  toggle($empty, false);
  $list.innerHTML = '';

  const filter = $search.value.toLowerCase().trim();
  const activeTags = state.activeTags;
  state.projects.forEach(p => {
    // Special tag: 'hidden' excludes the project entirely from the table.
    if ((p.tags || []).includes(HIDDEN_TAG)) return;
    if (filter && !p.name.toLowerCase().includes(filter) && !(p.framework || '').toLowerCase().includes(filter)
        && !(p.tags || []).some(t => t.toLowerCase().includes(filter))) return;
    if (activeTags.size > 0 && !(p.tags || []).some(t => activeTags.has(t))) return;
    $list.appendChild(createRow(p));
  });
}

/** Sort tags by saved order; new tags go at the end alphabetically. */
function sortedTags(allTags) {
  const order = state.settings.tag_order || [];
  const known = order.filter(t => allTags.includes(t));
  const knownSet = new Set(known);
  const extras = allTags.filter(t => !knownSet.has(t)).sort();
  return [...known, ...extras];
}

function renderTagBar() {
  // Don't rebuild while a drag is in progress — it would orphan the dragged
  // chip and clobber the user's in-flight reorder. Status events fire often
  // and would otherwise race the drag.
  if (_tagDrag && _tagDrag.active) return;

  if (!state.settings.tags_visible) {
    toggle($tagBar, false);
    return;
  }
  const counts = new Map();
  for (const p of state.projects) {
    if ((p.tags || []).includes(HIDDEN_TAG)) continue;
    for (const t of (p.tags || [])) counts.set(t, (counts.get(t) || 0) + 1);
  }
  // Drop active tags that no longer exist among visible projects.
  for (const t of [...state.activeTags]) {
    if (!counts.has(t)) state.activeTags.delete(t);
  }
  const allTags = sortedTags([...counts.keys()]);
  if (allTags.length === 0) {
    toggle($tagBar, false);
    return;
  }
  toggle($tagBar, true);
  $tagBarList.innerHTML = '';

  allTags.forEach(tag => {
    const count = counts.get(tag);
    const chip = el('button', 'cloud-tag' + (state.activeTags.has(tag) ? ' active' : ''));
    chip.style.setProperty('--tag-color', tagColor(tag));
    chip.dataset.tag = tag;
    chip.appendChild(document.createTextNode(tag));
    if (count > 1) {
      chip.appendChild(el('span', 'cloud-tag-count', String(count)));
    }
    chip.title = 'Click to filter, Ctrl/Cmd-click to add, drag to reorder';
    chip.addEventListener('click', e => {
      if (_tagDragMoved) return; // suppress click after drag
      if (e.ctrlKey || e.metaKey || e.shiftKey) {
        if (state.activeTags.has(tag)) state.activeTags.delete(tag);
        else state.activeTags.add(tag);
      } else {
        // Plain click: toggle exclusive selection.
        if (state.activeTags.size === 1 && state.activeTags.has(tag)) {
          state.activeTags.clear();
        } else {
          state.activeTags.clear();
          state.activeTags.add(tag);
        }
      }
      render();
    });
    attachTagDragHandlers(chip);
    $tagBarList.appendChild(chip);
  });

  toggle($tagClearBtn, state.activeTags.size > 0);
}

/** Apply the saved tags_visible setting to the top tag bar only (row tags stay). */
function applyTagsVisibility() {
  const visible = state.settings.tags_visible !== false;
  document.body.classList.toggle('tags-hidden', !visible);
  const btnTags = $('btn-toggle-tags');
  if (btnTags) {
    btnTags.classList.toggle('off', !visible);
    btnTags.title = visible ? 'Hide tags' : 'Show tags';
  }
}

$tagClearBtn.addEventListener('click', () => {
  state.activeTags.clear();
  render();
});

// ── Tag bar: drag-and-drop reorder ────────────────

let _tagDrag = null;
let _tagDragMoved = false;

function attachTagDragHandlers(chip) {
  chip.addEventListener('pointerdown', e => {
    if (e.button !== 0) return;
    _tagDrag = {
      el: chip,
      tag: chip.dataset.tag,
      startX: e.clientX,
      startY: e.clientY,
      active: false,
    };
    _tagDragMoved = false;
    document.addEventListener('pointermove', onTagDragMove);
    document.addEventListener('pointerup', onTagDragEnd);
    // Without this a cancelled gesture would leave _tagDrag set, and
    // renderTagBar() bails while a drag is active — the tag bar would stop
    // updating until restart.
    document.addEventListener('pointercancel', onTagDragEnd);
  });
}

function onTagDragMove(e) {
  if (!_tagDrag) return;
  if (!_tagDrag.active) {
    const dx = Math.abs(e.clientX - _tagDrag.startX);
    const dy = Math.abs(e.clientY - _tagDrag.startY);
    if (Math.max(dx, dy) < 5) return;
    _tagDrag.active = true;
    _tagDrag.el.classList.add('tag-dragging');
    document.body.classList.add('is-dragging-tag');
  }
  _tagDragMoved = true;

  // Find the chip we're hovering over.
  const chips = [...$tagBarList.querySelectorAll('.cloud-tag')];
  let target = null;
  for (const c of chips) {
    if (c === _tagDrag.el) continue;
    const r = c.getBoundingClientRect();
    if (e.clientX >= r.left && e.clientX <= r.right && e.clientY >= r.top - 8 && e.clientY <= r.bottom + 8) {
      target = c;
      break;
    }
  }
  if (target) {
    const r = target.getBoundingClientRect();
    const after = e.clientX > r.left + r.width / 2;
    if (after) target.after(_tagDrag.el);
    else target.before(_tagDrag.el);
  }
}

function onTagDragEnd() {
  document.removeEventListener('pointermove', onTagDragMove);
  document.removeEventListener('pointerup', onTagDragEnd);
  document.removeEventListener('pointercancel', onTagDragEnd);
  if (!_tagDrag) return;
  const wasActive = _tagDrag.active;
  _tagDrag.el.classList.remove('tag-dragging');
  document.body.classList.remove('is-dragging-tag');

  if (wasActive) {
    // Persist new order from current DOM order.
    const newOrder = [...$tagBarList.querySelectorAll('.cloud-tag')].map(c => c.dataset.tag);
    state.settings.tag_order = newOrder;
    api.saveSettings(state.settings).catch(() => {});
    // Re-render to keep colors stable & reapply state.
    render();
  }

  _tagDrag = null;
  // Clear suppression flag on next tick so the click that follows is swallowed.
  setTimeout(() => { _tagDragMoved = false; }, 0);
}

function createRow(p) {
  const s = getStatus(p.id);
  const missing = state.missingPaths.has(p.id);
  let cls = 'project-row';
  if (missing) cls += ' missing';
  else if (p.id === state.activeLogId) cls += ' active';
  if (p.pinned && !missing) cls += ' pinned';
  if (s.running && !missing) cls += ' running';
  const tr = el('tr', cls);
  tr.dataset.id = p.id;

  // Status dot
  const tdStatus = el('td');
  tdStatus.appendChild(el('span', 'status-dot ' + (missing ? 'error' : s.running ? 'running' : 'stopped')));

  // Name + framework
  const tdName = el('td');
  const nameWrap = el('div', 'name-cell');
  nameWrap.appendChild(el('span', 'project-name', p.name));
  if (p.pinned && !missing) {
    const pinSpan = el('span', 'pin-icon', '\u{1F4CC}');
    pinSpan.title = 'Click to unpin';
    pinSpan.addEventListener('click', e => {
      e.stopPropagation();
      p.pinned = false;
      ensurePinnedOrder();
      api.saveConfig(state.projects);
      render();
      toast(`${p.name} unpinned`, 'info', 2000);
    });
    nameWrap.appendChild(pinSpan);
  }
  if (p.framework) nameWrap.appendChild(el('span', 'framework-badge', p.framework));
  tdName.appendChild(nameWrap);

  if (missing) {
    const tdMissing = el('td');
    tdMissing.colSpan = 3; // tags + url + actions
    tdMissing.style.textAlign = 'right';
    const cluster = el('div', 'missing-actions');

    const relocateBtn = btn('relocate-btn', 'Relocate', async e => {
      e.stopPropagation();
      const folder = await api.pickFolder();
      if (folder) {
        // If anything is still running from the old (now-missing) path, kill
        // it before remapping — its cwd would otherwise be stale and it would
        // keep holding its port with no obvious link to the relocated project.
        if (getStatus(p.id).running) {
          try { await api.stopAll(p.id); } catch (_) {}
        }
        p.directory = folder;
        try {
          const scan = await api.scanProject(folder);
          if (scan.framework) p.framework = scan.framework;
        } catch (_) {}
        await api.saveConfig(state.projects);
        await checkProjectPaths();
        render();
      }
    });
    cluster.appendChild(relocateBtn);

    const deleteBtn = btn('delete-btn', 'Delete', e => {
      e.stopPropagation();
      showConfirm(`Remove "${p.name}"?`, async () => {
        const name = p.name;
        if (getStatus(p.id).running) {
          try { await api.stopAll(p.id); } catch (_) {}
        }
        state.projects = state.projects.filter(x => x.id !== p.id);
        delete state.statuses[p.id];
        await api.saveConfig(state.projects);
        try { await api.purgeProject(p.id); } catch (_) {}
        state.missingPaths.delete(p.id);
        if (state.activeLogId === p.id) closeLogPanel();
        render();
        toast(`${name} removed`, 'warn', 3000);
      });
    });
    cluster.appendChild(deleteBtn);

    tdMissing.appendChild(cluster);
    tr.append(tdStatus, tdName, tdMissing);
    return tr;
  }

  // Tags column
  const tdTags = el('td', 'col-tags-cell');
  if (p.tags && p.tags.length) {
    const tagsWrap = el('div', 'row-tags');
    p.tags.forEach(t => {
      if (t === HIDDEN_TAG) return; // never render the special tag
      const tagEl = el('span', 'row-tag' + (state.activeTags.has(t) ? ' active' : ''), t);
      tagEl.style.setProperty('--tag-color', tagColor(t));
      tagEl.title = `Filter by ${t} (Ctrl-click to add)`;
      tagEl.addEventListener('click', e => {
        e.stopPropagation();
        if (e.ctrlKey || e.metaKey || e.shiftKey) {
          if (state.activeTags.has(t)) state.activeTags.delete(t);
          else state.activeTags.add(t);
        } else {
          if (state.activeTags.size === 1 && state.activeTags.has(t)) {
            state.activeTags.clear();
          } else {
            state.activeTags.clear();
            state.activeTags.add(t);
          }
        }
        render();
      });
      tagsWrap.appendChild(tagEl);
    });
    tdTags.appendChild(tagsWrap);
  }

  // URL \u2014 entire cell is clickable for a bigger hit target
  const tdUrl = el('td', 'col-url-cell');
  if (s.url) {
    tdUrl.classList.add('clickable');
    tdUrl.title = s.url;
    tdUrl.appendChild(el('span', 'url-link', s.url.replace('http://', '')));
    tdUrl.addEventListener('click', e => { e.stopPropagation(); api.openInBrowser(s.url); });
  } else {
    tdUrl.appendChild(el('span', 'url-placeholder', s.running ? 'detecting...' : '\u2014'));
  }

  // Right-side actions cluster: play + 3-dots together at the end
  const tdActions = el('td', 'actions-cell');
  const cluster = el('div', 'actions-cluster');

  const playBtn = btn('play-btn' + (s.running ? ' running' : ''), null, e => {
    e.stopPropagation();
    if (s.running) {
      api.stopAll(p.id);
    } else {
      const devCmd = (p.commands || []).find(c =>
        ['dev', 'start', 'serve'].includes(c.label)
      ) || (p.commands || [])[0];
      if (devCmd) runCommand(p.id, devCmd.label, devCmd.cmd, p.directory, p.env);
    }
  });
  playBtn.innerHTML = s.running ? '&#9632;' : '&#9654;';
  playBtn.title = s.running ? 'Stop all' : 'Start dev';
  cluster.appendChild(playBtn);

  const dots = btn('dots-btn', null, e => { e.stopPropagation(); openContextMenu(p.id, e); });
  dots.innerHTML = '&#8942;';
  cluster.appendChild(dots);

  tdActions.appendChild(cluster);

  tr.append(tdStatus, tdName, tdTags, tdUrl, tdActions);
  tr.addEventListener('click', () => openLogPanel(p.id));
  return tr;
}

// ── Run command (shared) ───────────────────────────

export async function runCommand(id, label, cmd, cwd, env = []) {
  // Only stop if the SAME command is already running (restart).
  // stop_process now awaits the kill, so the port is released by the time
  // it returns — no extra grace period needed.
  const cs = getCmdStatus(id, label);
  if (cs.running) {
    try { await api.stopProcess(id, label); } catch (_) {}
  }

  const $logOut = $('log-output');
  if (id === state.activeLogId && label === state.activeLogTab) {
    resetLogLines();
    $logOut.innerHTML = '';
    appendLogLine($logOut, `$ ${cmd}`, 'info');
  }

  try {
    await api.startProcess(id, cmd, label, cwd, env);
  } catch (err) {
    if (id === state.activeLogId && label === state.activeLogTab) {
      appendLogLine($logOut, `Error: ${err}`, 'stderr');
    }
    toast(`Failed to start ${label}: ${err}`, 'error', 5000);
  }
}

// ── Sort helpers ──────────────────────────────────

/** Ensure pinned projects come first in the array (stable). */
export function ensurePinnedOrder() {
  const pinned = state.projects.filter(p => p.pinned);
  const normal = state.projects.filter(p => !p.pinned);
  state.projects = [...pinned, ...normal];
}

// ── Drag-and-drop reorder ─────────────────────────

const DRAG_THRESHOLD = 6; // px before drag activates
let _drag = null;

function onRowPointerDown(e) {
  // Ignore if clicking interactive elements
  if (e.target.closest('button, a, .play-btn, .dots-btn, .relocate-btn, .url-link, .col-url-cell.clickable, .pin-icon, .row-tag')) return;
  if (e.button !== 0) return; // left button only

  const tr = e.target.closest('.project-row');
  if (!tr) return;

  const id = tr.dataset.id;
  const proj = state.projects.find(p => p.id === id);
  if (!proj) return;

  _drag = {
    id,
    el: tr,
    isPinned: !!proj.pinned,
    startY: e.clientY,
    startX: e.clientX,
    active: false,
    ghost: null,
    indicator: null,
    offsetY: 0,
    dropTarget: null,
  };

  document.addEventListener('pointermove', onDragMove);
  document.addEventListener('pointerup', onDragEnd);
  document.addEventListener('pointercancel', cleanupDrag);
}

function activateDrag() {
  _drag.active = true;
  _drag.el.classList.add('dragging');
  document.body.classList.add('is-dragging');

  // Create ghost (wrap cloned row in a table so it renders properly)
  const rect = _drag.el.getBoundingClientRect();
  const ghost = document.createElement('table');
  ghost.className = 'drag-ghost';
  ghost.style.width = rect.width + 'px';
  ghost.style.top = rect.top + 'px';
  ghost.style.left = rect.left + 'px';
  const tbody = document.createElement('tbody');
  const clonedRow = _drag.el.cloneNode(true);
  clonedRow.classList.remove('dragging');
  tbody.appendChild(clonedRow);
  ghost.appendChild(tbody);
  document.body.appendChild(ghost);
  _drag.ghost = ghost;
  _drag.offsetY = _drag.startY - rect.top;

  // Create drop indicator line
  const ind = document.createElement('div');
  ind.className = 'drop-indicator';
  document.body.appendChild(ind);
  _drag.indicator = ind;
}

function onDragMove(e) {
  if (!_drag) return;

  if (!_drag.active) {
    const dy = Math.abs(e.clientY - _drag.startY);
    const dx = Math.abs(e.clientX - _drag.startX);
    if (dy < DRAG_THRESHOLD) return;
    if (dx > dy * 2) { cleanupDrag(); return; } // horizontal move, abort
    activateDrag();
  }

  // Move ghost
  _drag.ghost.style.top = (e.clientY - _drag.offsetY) + 'px';

  // Find closest row in the same group (pinned/normal)
  const rows = [...$list.querySelectorAll('.project-row:not(.dragging)')];
  let best = null, bestDist = Infinity, before = true;

  for (const row of rows) {
    const rp = state.projects.find(p => p.id === row.dataset.id);
    if (!rp) continue;
    // Only allow drop within same group
    if (!!rp.pinned !== _drag.isPinned) continue;

    const rect = row.getBoundingClientRect();
    const mid = rect.top + rect.height / 2;
    const dist = Math.abs(e.clientY - mid);
    if (dist < bestDist) {
      bestDist = dist;
      best = row;
      before = e.clientY < mid;
    }
  }

  if (best) {
    const rect = best.getBoundingClientRect();
    const tableRect = $list.closest('table').getBoundingClientRect();
    const y = before ? rect.top : rect.bottom;
    _drag.indicator.style.display = 'block';
    _drag.indicator.style.top = y + 'px';
    _drag.indicator.style.left = tableRect.left + 'px';
    _drag.indicator.style.width = tableRect.width + 'px';
    _drag.dropTarget = { id: best.dataset.id, before };
  } else {
    _drag.indicator.style.display = 'none';
    _drag.dropTarget = null;
  }
}

function onDragEnd() {
  document.removeEventListener('pointermove', onDragMove);
  document.removeEventListener('pointerup', onDragEnd);
  document.removeEventListener('pointercancel', cleanupDrag);
  if (!_drag) return;

  if (_drag.active && _drag.dropTarget) {
    const { id: targetId, before } = _drag.dropTarget;
    const fromIdx = state.projects.findIndex(p => p.id === _drag.id);
    let toIdx = state.projects.findIndex(p => p.id === targetId);
    if (fromIdx !== -1 && toIdx !== -1 && fromIdx !== toIdx) {
      const [moved] = state.projects.splice(fromIdx, 1);
      // Recalculate toIdx after removal
      toIdx = state.projects.findIndex(p => p.id === targetId);
      const insertIdx = before ? toIdx : toIdx + 1;
      state.projects.splice(insertIdx, 0, moved);
      api.saveConfig(state.projects);
      render();
    }
  }

  if (_drag.active) {
    // Suppress the click that would open logs
    const row = _drag.el;
    const suppress = e => { e.stopImmediatePropagation(); e.preventDefault(); };
    row.addEventListener('click', suppress, { capture: true, once: true });
    setTimeout(() => row.removeEventListener('click', suppress, { capture: true }), 50);
  }

  cleanupDrag();
}

function cleanupDrag() {
  if (!_drag) return;
  _drag.ghost?.remove();
  _drag.indicator?.remove();
  _drag.el?.classList.remove('dragging');
  document.body.classList.remove('is-dragging');
  document.removeEventListener('pointermove', onDragMove);
  document.removeEventListener('pointerup', onDragEnd);
  document.removeEventListener('pointercancel', cleanupDrag);
  _drag = null;
}

$list.addEventListener('pointerdown', onRowPointerDown);

// ── Search ─────────────────────────────────────────

function updateSearchClear() {
  toggle($searchClear, $search.value.length > 0);
}
$search.addEventListener('input', () => {
  updateSearchClear();
  render();
});
$searchClear.addEventListener('click', () => {
  $search.value = '';
  updateSearchClear();
  render();
  $search.focus();
});
