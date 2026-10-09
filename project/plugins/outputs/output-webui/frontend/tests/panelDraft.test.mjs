import { test } from 'node:test';
import assert from 'node:assert/strict';
import { clone, patchFrom, conflictingFields } from '../src/panelDraft.ts';

test('field patches preserve concurrent geometry and unrelated panel settings', () => {
  const base = { title: 'Logs', left: 20, format: 'text', streams: [{ owner: 'a', alias: 'b' }] };
  const form = clone(base); form.format = 'hex';
  const patch = patchFrom(base, form, Object.keys(base));
  assert.deepEqual(patch, { format: 'hex' });
  assert.deepEqual(conflictingFields(base, patch, { ...base, title: 'Remote', left: 200 }), []);
});
test('same-field edits conflict, identical outcomes and reordered object keys do not', () => {
  const base = { title: 'Logs', streams: [{ owner: 'a', alias: 'b' }] };
  assert.deepEqual(conflictingFields(base, { title: 'Mine' }, { ...base, title: 'Theirs' }), ['title']);
  assert.deepEqual(conflictingFields(base, { title: 'Mine' }, { ...base, title: 'Mine' }), []);
  assert.deepEqual(patchFrom(base, { ...base, streams: [{ alias: 'b', owner: 'a' }] }, ['streams']), {});
});
test('drafts do not share mutable arrays with the latest server snapshot', () => {
  const base = { channels: ['stdout'], y_min: undefined };
  const form = clone(base); form.channels.push('stderr');
  assert.deepEqual(base.channels, ['stdout']);
  assert.equal(clone(undefined), undefined);
});
