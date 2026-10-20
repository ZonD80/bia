// Markdown → HTML for chat bubbles (the model writes markdown). No DOM and no app state,
// so tests/integration/web-md-test.js runs it under node. opt.paths: link absolute paths
// (root's file manager).
'use strict';
const BiaMd = (() => {
  const esc = s => String(s).replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));

  function md(src, opt = {}) {
    const inl = s => inline(s, opt);
    const lines = String(src).replace(/\r\n?/g, '\n').split('\n');
    const out = [];
    let i = 0;
    const isFence = l => /^\s*```/.test(l);
    const isList = l => /^\s*([-*+•]|\d+[.)])\s+/.test(l);
    const isTable = (l, n) => /^\s*\|.*\|\s*$/.test(l) && n !== undefined && /^\s*\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$/.test(n);
    const special = (k) => isFence(lines[k]) || /^#{1,6}\s/.test(lines[k]) || isList(lines[k]) || /^\s*>/.test(lines[k]) || /^\s*(---|\*\*\*|___)\s*$/.test(lines[k]) || isTable(lines[k], lines[k + 1]);
    while (i < lines.length) {
      const l = lines[i];
      const fence = /^\s*```\s*([\w+#.-]*)/.exec(l);
      if (fence) {
        const buf = []; i++;
        while (i < lines.length && !/^\s*```\s*$/.test(lines[i])) buf.push(lines[i++]);
        i++;
        out.push(`<pre><span class="lang" title="Copy">${esc(fence[1] || 'copy')}</span><code>${esc(buf.join('\n'))}</code></pre>`);
        continue;
      }
      if (/^\s*(---|\*\*\*|___)\s*$/.test(l)) { out.push('<hr>'); i++; continue; }
      const hd = /^(#{1,6})\s+(.*)$/.exec(l);
      if (hd) { out.push(`<h3>${inl(hd[2])}</h3>`); i++; continue; }
      if (/^\s*>/.test(l)) {
        const buf = [];
        while (i < lines.length && /^\s*>/.test(lines[i])) buf.push(lines[i++].replace(/^\s*>\s?/, ''));
        out.push(`<blockquote>${md(buf.join('\n'), opt)}</blockquote>`);
        continue;
      }
      if (isTable(l, lines[i + 1])) {
        const row = s => s.trim().replace(/^\||\|$/g, '').split('|').map(x => x.trim());
        const head = row(l); i += 2;
        const body = [];
        while (i < lines.length && /^\s*\|.*\|\s*$/.test(lines[i])) body.push(row(lines[i++]));
        out.push(`<table><thead><tr>${head.map(c => `<th>${inl(c)}</th>`).join('')}</tr></thead><tbody>${body.map(r => `<tr>${r.map(c => `<td>${inl(c)}</td>`).join('')}</tr>`).join('')}</tbody></table>`);
        continue;
      }
      if (isList(l)) {
        const ordered = /^\s*\d/.test(l);
        const start = ordered ? parseInt(l, 10) : 1;
        const items = [];
        while (i < lines.length) {
          const m = /^\s*([-*+•]|\d+[.)])\s+(.*)$/.exec(lines[i]);
          if (m) { items.push(m[2]); i++; continue; }
          if (items.length && /^\s{2,}\S/.test(lines[i])) { items[items.length - 1] += '\n' + lines[i].trim(); i++; continue; }
          break;
        }
        const tag = ordered ? 'ol' : 'ul';
        out.push(`<${tag}${ordered && start !== 1 ? ` start="${start}"` : ''}>${items.map(x => `<li>${x.split('\n').map(inl).join('<br>')}</li>`).join('')}</${tag}>`);
        continue;
      }
      if (!l.trim()) { i++; continue; }
      const buf = [l]; i++;
      while (i < lines.length && lines[i].trim() && !special(i)) buf.push(lines[i++]);
      out.push(`<p>${buf.map(inl).join('<br>')}</p>`);
    }
    return out.join('');
  }
  function inline(s, opt = {}) {
    // Code spans, links and paths become placeholders first (a link or path never ends in
  // * or ~, which close the emphasis around it), so emphasis can span them
    // (**see `drive/v3/about`:** is one bold run) and nothing inside them is formatted.
    const hold = [];
    const keep = x => { hold.push(x); return `\u0000${hold.length - 1}\u0000`; };
    let t = String(s).replace(/\u0000/g, '');
    t = t.replace(/`([^`\n]+)`/g, (m, c) => keep(`<code>${esc(c)}</code>`));
    t = t.replace(/\[([^\]\n]+)\]\(((?:https?:\/\/|\/)[^)\s]*)\)/g, (m, a, u) => keep(`<a href="${esc(u)}" target="_blank" rel="noopener noreferrer">${esc(a)}</a>`));
    t = t.replace(/\bhttps?:\/\/[^\s<>"'`\u0000]*[^\s<>"'`.,;:!?)\]\u0000*~]/g, u => keep(`<a href="${esc(u)}" target="_blank" rel="noopener noreferrer">${esc(u)}</a>`));
    if (opt.paths) t = t.replace(/(^|[\s(])(\/(?:home|tmp|etc|var|root|usr|opt|srv|mnt|media|run)\/[^\s<>"'`)\u0000]*[^\s<>"'`.,;:!?)\]\u0000*~])/g,
      (m, pre, pth) => pre + keep(`<a href="#" class="plink" data-path="${esc(pth)}">${esc(pth)}</a>`));
    t = esc(t)
      .replace(/\*\*(?=\S)([\s\S]*?\S)\*\*/g, '<b>$1</b>')
      .replace(/__(?=\S)([\s\S]*?\S)__/g, '<u>$1</u>')
      .replace(/~~(?=\S)([\s\S]*?\S)~~/g, '<s>$1</s>')
      .replace(/\|\|(?=\S)([\s\S]*?\S)\|\|/g, '<span class="spoiler">$1</span>')
      .replace(/(^|[^\w*])\*(?=\S)([^*\n]*?\S)\*(?![\w*])/g, '$1<i>$2</i>')
      .replace(/(^|[^\w])_(?=\S)([^_\n]*?\S)_(?!\w)/g, '$1<i>$2</i>');
    return t.replace(/\u0000(\d+)\u0000/g, (m, n) => hold[n]);
  }

  return { md, inline };
})();
if (typeof module === 'object' && module.exports) module.exports = BiaMd;
