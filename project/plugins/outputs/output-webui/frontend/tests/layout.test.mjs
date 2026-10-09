import { test } from 'node:test';
import assert from 'node:assert/strict';
import { place, compact, available, snapMove } from '../src/layout.ts';
const rect = (id, left = 24, top = 24, width = 320, height = 220) => ({ id, left, top, panel_width: width, panel_height: height });
test('new panels fill a row with an eight pixel gap, then wrap', () => {
  const a = rect('a'), b = place(rect('b'), [a], { x: 24, y: 24 }, 648);
  assert.deepEqual([b.left, b.top], [352, 24]);
  const c = place(rect('c'), [a, b], { x: 24, y: 24 }, 648);
  assert.deepEqual([c.left, c.top], [24, 252]);
});
test('dragging snaps to peer edges and cannot land underneath another panel', () => {
  const a = rect('a', 0, 0), b = rect('b', 332, 3);
  const snapped = snapMove(b, [a], 8, true, false);
  assert.deepEqual([snapped.left, snapped.top], [328, 0]);
  const collision = snapMove(rect('b', 200, 20), [a], 8, false, false);
  assert.equal(available(collision, [a]), true);
  assert.deepEqual(snapMove(rect('b', 200, 20), [a], 8, false, true), rect('b', 200, 20));
});
test('compact preserves locked geometry, skips hidden panels and avoids all placed panels', () => {
  const panels = [{ ...rect('locked', 100, 60), locked: true }, rect('a'), rect('b'), { ...rect('hidden'), hidden: true }];
  const result = compact(panels, 1000);
  assert.deepEqual(result[0], panels[0]);
  assert.equal(result.length, 3);
  for (const p of result) assert.equal(available(p, result), true);
});
test('negative coordinates and large panels terminate with valid placement', () => {
  const others = [rect('a', -500, -220, 900, 440)];
  const p = place(rect('b', 0, 0, 4000, 4000), others, { x: 24, y: 24 }, 320);
  assert.equal(available(p, others), true);
});
