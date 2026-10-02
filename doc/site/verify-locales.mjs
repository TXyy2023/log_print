// Exercise the generated pages and shipped search indexes, not just source labels.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import MiniSearch from 'minisearch';

const home = path.dirname(fileURLToPath(import.meta.url));
const dist = path.resolve(process.argv[2] || path.join(home, 'dist/public'));
const base = process.argv[3] || '/';
assert.match(base, /^\/(?:[A-Za-z0-9_~-]+(?:\.[A-Za-z0-9_~-]+)*\/)*$/);
const allow = JSON.parse(fs.readFileSync(path.join(home, 'public-pages.json')));
const pages = allow.filter(p => p.endsWith('.md') && !p.startsWith('zh/'));
assert.equal(new Set(allow).size, allow.length, 'duplicate allowlist entries');
assert.deepEqual(allow.filter(p => p.startsWith('zh/')).sort(), pages.map(p => 'zh/' + p).sort(), 'each public page needs a Chinese counterpart');
const navigation = JSON.parse(fs.readFileSync(path.join(home, 'public-navigation.json')));
const route = p => p.replace(/(^|\/)index\.md$/, '$1').replace(/\.md$/, '.html');
for (const page of pages) {
  for (const zh of [false, true]) {
    const rel = (zh ? 'zh/' : '') + page;
    const html = fs.readFileSync(path.join(dist, rel.replace(/\.md$/, '.html')), 'utf8');
    assert.match(html, new RegExp(`<html lang="${zh ? 'zh-CN' : 'en'}"`), rel + ': language');
    const ids = [...html.matchAll(/\bid="([^"]+)"/g)].map(match => match[1]);
    assert.equal(new Set(ids).size, ids.length, rel + ': duplicate HTML anchor');
    const counterpart = base + route((zh ? '' : 'zh/') + page);
    assert.ok(html.includes(`href="${counterpart}"`), rel + ': missing corresponding language link');
    const otherHtml = fs.readFileSync(path.join(dist, ((zh ? '' : 'zh/') + page).replace(/\.md$/, '.html')), 'utf8');
    for (const [, id] of html.matchAll(/<h[1-6]\b[^>]* id="([^"]+)"/g)) {
      assert.ok(otherHtml.includes(`id="${id}"`), rel + ': translated heading anchor missing: ' + id);
    }
    const h1 = html.match(/<h1\b[^>]*>(.*?)<\/h1>/s)?.[1];
    assert.ok(h1, rel + ': missing title');
    assert.equal(/\p{Script=Han}/u.test(h1), zh, rel + ': title language');
    const sidebar = html.match(/<aside class="VPSidebar"[\s\S]*?<\/aside>/)?.[0];
    assert.ok(sidebar, rel + ': missing sidebar');
    for (const group of navigation) {
      assert.ok(group.text[zh ? 'zh' : 'en'], 'missing group translation');
      for (const item of group.items) {
        assert.ok(item.text[zh ? 'zh' : 'en'], 'missing navigation translation');
        const target = base + (zh ? 'zh/' : '') + route(item.page);
        assert.ok(sidebar.includes(`href="${target}"`), rel + ': sidebar leaves its locale or misses ' + target);
      }
    }
  }
}
const chunks = path.join(dist, 'assets/chunks');
const segmenter = new Intl.Segmenter('zh', { granularity: 'word' });
const tokenize = text => Array.from(segmenter.segment(text), s => s.segment).filter(s => /\p{L}|\p{N}/u.test(s));
for (const locale of ['root', 'zh']) {
  const matches = fs.readdirSync(chunks).filter(p => p.startsWith('@localSearchIndex' + locale + '.') && p.endsWith('.js'));
  assert.equal(matches.length, 1, locale + ': missing or stale search index');
  const json = (await import(pathToFileURL(path.join(chunks, matches[0])).href)).default;
  const data = JSON.parse(json);
  const ids = Object.values(data.documentIds);
  const expected = pages.map(p => base + route((locale === 'zh' ? 'zh/' : '') + p));
  assert.deepEqual([...new Set(ids.map(id => id.split('#')[0]))].sort(), expected.sort(), locale + ': search index missing pages or leaking another locale');
  const search = MiniSearch.loadJSON(json, {fields: ['title', 'titles', 'text'], storeFields: ['title', 'titles'], tokenize});
  const cases = locale === 'zh' ? [['归档', 'plugins/output-file.html'], ['终端', 'plugins/output-tui.html']] : [['archive', 'plugins/output-file.html'], ['terminal', 'plugins/output-tui.html']];
  for (const [query, target] of cases) {
    const results = search.search(query, {prefix: true, fuzzy: 0.2});
    assert.ok(results.some(r => r.id.split('#')[0] === base + (locale === 'zh' ? 'zh/' : '') + target), locale + ': search failed for ' + query);
  }
}
console.log(`Locale validation PASS: ${pages.length} page pairs, English default, corresponding links, localized sidebars and both search indexes (${base}).`);
