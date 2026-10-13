import test from 'node:test';
import assert from 'node:assert/strict';
import { highlight } from '../src/logText.ts';
test('literal highlighter preserves hostile markup, Unicode and metacharacters', () => {
  const text = '<script>中文.*中文</script>';
  const parts = highlight(text, '.*');
  assert.equal(parts.map(p => p.text).join(''), text);
  assert.deepEqual(parts.filter(p => p.match).map(p => p.text), ['.*']);
  assert.deepEqual(highlight(text, ''), [{ text, match: false }]);
});
test('pathological long lines keep full text without unbounded highlight nodes', () => {
  const text = 'a'.repeat(65536);
  const parts = highlight(text, 'a');
  assert.equal(parts.map(p => p.text).join(''), text);
  assert.ok(parts.length < 600);
});
