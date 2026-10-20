// Markdown rendering of the web app's chat bubbles (source/crates/bia-mcp-web/web/md.js).
//
//   node tests/integration/web-md-test.js
'use strict';
const path = require('path');
const { md } = require(path.join(__dirname, '../../source/crates/bia-mcp-web/web/md.js'));

const link = u => `<a href="${u}" target="_blank" rel="noopener noreferrer">`;
const cases = [
  // emphasis around code: once split into separate runs, so the ** stayed visible
  ['**Квота (через API, `drive/v3/about`):**', '<p><b>Квота (через API, <code>drive/v3/about</code>):</b></p>'],
  ['- Лимит: **16 106 127 360 Б = 15.0 ГБ**', '<ul><li>Лимит: <b>16 106 127 360 Б = 15.0 ГБ</b></li></ul>'],
  ['_italic `x_y` end_', '<p><i>italic <code>x_y</code> end</i></p>'],
  ['~~old `v1`~~ new', '<p><s>old <code>v1</code></s> new</p>'],
  // nothing inside code is formatted
  ['`**not bold**` and **bold `a*b`**', '<p><code>**not bold**</code> and <b>bold <code>a*b</code></b></p>'],
  // links inside emphasis; underscores in URLs and paths stay
  ['**[docs](https://x.io/a_b_c) here**', `<p><b>${link('https://x.io/a_b_c')}docs</a> here</b></p>`],
  ['see https://x.io/a_b_c.', `<p>see ${link('https://x.io/a_b_c')}https://x.io/a_b_c</a>.</p>`],
  ['**see https://x.io/p**', `<p><b>see ${link('https://x.io/p')}https://x.io/p</a></b></p>`],
  ['in my_var_name and 2*3*4', '<p>in my_var_name and 2*3*4</p>'],
  // blocks
  ['# Title **x**', '<h3>Title <b>x</b></h3>'],
  ['> **quoted** `c`', '<blockquote><p><b>quoted</b> <code>c</code></p></blockquote>'],
  ['```sh\necho **x**\n```', '<pre><span class="lang" title="Copy">sh</span><code>echo **x**</code></pre>'],
  ['| a | b |\n|---|---|\n| **1** | `2` |', '<table><thead><tr><th>a</th><th>b</th></tr></thead><tbody><tr><td><b>1</b></td><td><code>2</code></td></tr></tbody></table>'],
  // HTML is escaped everywhere; a NUL cannot forge a placeholder
  ['a <b>raw</b> & `<script>`', '<p>a &lt;b&gt;raw&lt;/b&gt; &amp; <code>&lt;script&gt;</code></p>'],
  ['x \u00000\u0000 y `z`', '<p>x 0 y <code>z</code></p>'],
];
const pathCases = [
  ['see /home/bia/files/my_file_name.txt now', '<p>see <a href="#" class="plink" data-path="/home/bia/files/my_file_name.txt">/home/bia/files/my_file_name.txt</a> now</p>'],
  ['**`/etc/x` and /tmp/a_b**', '<p><b><code>/etc/x</code> and <a href="#" class="plink" data-path="/tmp/a_b">/tmp/a_b</a></b></p>'],
];

let bad = 0;
const check = (src, want, opt) => {
  const got = md(src, opt);
  if (got === want) {
    console.log('ok   ' + JSON.stringify(src));
  } else {
    bad++;
    console.log('FAIL ' + JSON.stringify(src) + '\n  got  ' + got + '\n  want ' + want);
  }
};
for (const [src, want] of cases) check(src, want);
for (const [src, want] of pathCases) check(src, want, { paths: true });
check('see /home/bia/x', '<p>see /home/bia/x</p>');
console.log('RESULT', bad ? `FAILED (${bad})` : 'PASSED');
process.exit(bad ? 1 : 0);
