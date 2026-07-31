// ── ANSI escape code → HTML converter ──────────────
// Converts terminal ANSI SGR sequences to styled <span> elements.
// Supports: standard 16 colors, 256-color, 24-bit RGB, bold, dim, italic, underline.

// GitHub-dark-themed 16-color palette
const C16 = [
  '#6e7681', '#f85149', '#3fb950', '#d29922', '#58a6ff', '#bc8cff', '#39d2c0', '#b1bac4',
  '#8b949e', '#ff7b72', '#56d364', '#e3b341', '#79c0ff', '#d2a8ff', '#56d4dd', '#f0f6fc',
];

/** Clamp an SGR parameter to a 0-255 colour channel. Guards the inline style
 *  below — parameters are raw text from the log and must never reach the
 *  style attribute unvalidated. */
function chan(v) {
  const n = parseInt(v, 10);
  return isNaN(n) ? 0 : Math.max(0, Math.min(255, n));
}

function c256(n) {
  if (n < 16) return C16[n];
  if (n < 232) {
    n -= 16;
    return `rgb(${Math.floor(n / 36) * 51},${Math.floor((n % 36) / 6) * 51},${(n % 6) * 51})`;
  }
  const v = 8 + (n - 232) * 10;
  return `rgb(${v},${v},${v})`;
}

/**
 * Convert a string containing ANSI escape codes to HTML with inline styles.
 * Only SGR (Select Graphic Rendition) sequences are processed; others are stripped.
 * Text is HTML-escaped to prevent XSS.
 */
export function ansiToHtml(raw) {
  let out = '';
  let i = 0;
  let fg = null, bg = null;
  let bold = false, dim = false, italic = false, underline = false;
  let open = false;

  while (i < raw.length) {
    const ch = raw.charCodeAt(i);

    // ── ESC [ ... <letter>  (CSI sequence) ──
    if (ch === 0x1b && i + 1 < raw.length && raw.charCodeAt(i + 1) === 0x5b) {
      let j = i + 2;
      while (j < raw.length) {
        const c = raw.charCodeAt(j);
        if (c >= 0x40 && c <= 0x7e) break;
        j++;
      }
      if (j >= raw.length) { i = j; break; }
      const cmd = raw[j];
      const paramStr = raw.substring(i + 2, j);
      i = j + 1;
      if (cmd !== 'm') continue; // only handle SGR

      // Close previous span
      if (open) { out += '</span>'; open = false; }

      // Process SGR parameters
      const params = paramStr.split(';');
      for (let k = 0; k < params.length; k++) {
        const p = parseInt(params[k], 10);
        if (isNaN(p) || p === 0) { fg = bg = null; bold = dim = italic = underline = false; }
        else if (p === 1) bold = true;
        else if (p === 2) dim = true;
        else if (p === 3) italic = true;
        else if (p === 4) underline = true;
        else if (p === 22) { bold = false; dim = false; }
        else if (p === 23) italic = false;
        else if (p === 24) underline = false;
        else if (p >= 30 && p <= 37) fg = C16[p - 30];
        else if (p === 38) {
          const m = parseInt(params[k + 1], 10);
          if (m === 5 && k + 2 < params.length) { fg = c256(parseInt(params[k + 2], 10) || 0); k += 2; }
          else if (m === 2 && k + 4 < params.length) { fg = `rgb(${chan(params[k+2])},${chan(params[k+3])},${chan(params[k+4])})`; k += 4; }
        }
        else if (p === 39) fg = null;
        else if (p >= 40 && p <= 47) bg = C16[p - 40];
        else if (p === 48) {
          const m = parseInt(params[k + 1], 10);
          if (m === 5 && k + 2 < params.length) { bg = c256(parseInt(params[k + 2], 10) || 0); k += 2; }
          else if (m === 2 && k + 4 < params.length) { bg = `rgb(${chan(params[k+2])},${chan(params[k+3])},${chan(params[k+4])})`; k += 4; }
        }
        else if (p === 49) bg = null;
        else if (p >= 90 && p <= 97) fg = C16[p - 82];
        else if (p >= 100 && p <= 107) bg = C16[p - 92];
      }

      // Open new span if any style is active
      const s = [];
      if (fg) s.push('color:' + fg);
      if (bg) s.push('background:' + bg);
      if (bold) s.push('font-weight:700');
      if (dim) s.push('opacity:.6');
      if (italic) s.push('font-style:italic');
      if (underline) s.push('text-decoration:underline');
      if (s.length) { out += '<span style="' + s.join(';') + '">'; open = true; }
      continue;
    }

    // ── Other ESC sequences — skip ESC + next char ──
    if (ch === 0x1b) { i += 2; continue; }

    // ── Strip \r ──
    if (ch === 0x0d) { i++; continue; }

    // ── Regular text — HTML-escape ──
    if (ch === 0x3c) out += '&lt;';
    else if (ch === 0x3e) out += '&gt;';
    else if (ch === 0x26) out += '&amp;';
    else out += raw[i];
    i++;
  }

  if (open) out += '</span>';
  return out;
}
