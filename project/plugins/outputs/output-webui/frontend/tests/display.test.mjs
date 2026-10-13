import test from 'node:test';
import assert from 'node:assert/strict';
import { clockTime, decodeName, fileSize, hasGaps, rowKey } from '../src/display.ts';
test('stable selection distinguishes epochs and offsets without rounding large sequence numbers', () => {
  const row = { stream: 'stream', epoch: 'epoch', seq: '18446744073709551610', offset: 6 };
  assert.notEqual(rowKey(row), rowKey({ ...row, offset: 7 }));
  assert.notEqual(rowKey(row), rowKey({ ...row, epoch: 'next' }));
  assert.ok(rowKey(row).includes(row.seq));
});
test('coverage does not label missing prefixes or discontinuous memory as complete', () => {
  assert.equal(hasGaps({ streams: [{ uncovered_prefix: { first: '1', last: '20' } }] }), true);
  assert.equal(hasGaps({ streams: [{ memory: { range_count: 2 } }] }), true);
  assert.equal(hasGaps({ streams: [{ memory: { range_count: 1 } }] }), false);
  assert.equal(hasGaps(null), false);
});
test('display handles encoded filenames, empty files and invalid times', () => {
  assert.equal(decodeName('%E4%B8%AD%E6%96%87.log'), '中文.log');
  assert.equal(decodeName('bad%escape.log'), 'bad%escape.log');
  assert.equal(fileSize('0'), '0 B');
  assert.equal(fileSize('26'), '26 B');
  assert.equal(clockTime('invalid'), '—');
});
