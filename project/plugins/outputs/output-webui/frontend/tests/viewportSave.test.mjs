import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createViewportSaver } from '../src/viewportSave.ts';

const view = (zoom) => ({ x: 24, y: 24, zoom });
const drain = async () => { for (let i = 0; i < 10; i++) await Promise.resolve(); };
function fixture(t) {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  let saved = view(1), pending = 0, reconciles = 0;
  const writes = [];
  const saver = createViewportSaver({
    read: () => saved,
    write: (value, base) => new Promise((resolve, reject) => {
      writes.push({ value, base, reject, resolve: () => { saved = value; resolve(); } });
    }),
    reconcile: () => reconciles++,
    pending: (delta) => { pending += delta; },
  });
  return { saver, writes, get pending() { return pending; },
    get reconciles() { return reconciles; },
    gesture(zoom) { saver.begin(); saver.change(view(zoom)); saver.end(view(zoom)); },
    async idle() { t.mock.timers.tick(150); await drain(); },
  };
}

test('continuous pinch saves only its final viewport and stays pending during debounce', async (t) => {
  const f = fixture(t);
  for (let i = 1; i <= 40; i++) { f.gesture(1 + i / 100); t.mock.timers.tick(8); }
  assert.equal(f.pending, 1);
  assert.equal(f.writes.length, 0);
  await f.idle();
  assert.equal(f.writes.length, 1);
  assert.deepEqual(f.writes[0].value, view(1.4));
  f.writes[0].resolve(); await drain();
  assert.equal(f.pending, 0);
  assert.equal(f.reconciles, 1);
});

test('slow saves retain only the latest view and never rewind an active gesture', async (t) => {
  const f = fixture(t);
  f.gesture(1.2); await f.idle();
  f.gesture(1.3); f.gesture(1.4); await f.idle();
  f.saver.begin(); f.saver.change(view(1.5));
  f.writes[0].resolve(); await drain();
  assert.equal(f.reconciles, 0);
  assert.equal(f.writes.length, 1);
  f.saver.end(view(1.5)); await f.idle();
  assert.equal(f.writes.length, 2);
  assert.deepEqual(f.writes[1].base, view(1.2));
  assert.deepEqual(f.writes[1].value, view(1.5));
  f.writes[1].resolve(); await drain();
  assert.equal(f.pending, 0);
  assert.equal(f.reconciles, 1);
});

test('rejection drops the remaining gesture and allows a fresh gesture', async (t) => {
  const f = fixture(t);
  f.gesture(1.2); await f.idle();
  f.gesture(1.3);
  f.writes[0].reject(new Error('conflict')); await drain();
  f.gesture(1.4); await f.idle();
  assert.equal(f.writes.length, 1);
  assert.equal(f.pending, 0);
  assert.equal(f.reconciles, 1);
  f.gesture(1.5); await f.idle();
  assert.equal(f.writes.length, 2);
  assert.deepEqual(f.writes[1].base, view(1));
  f.writes[1].resolve(); await drain();
});

test('unmount flushes the last view after an in-flight save without reconciling the new canvas', async (t) => {
  const f = fixture(t);
  f.gesture(1.2); await f.idle();
  f.saver.begin(); f.saver.change(view(1.6));
  f.saver.dispose();
  f.writes[0].resolve(); await drain();
  assert.equal(f.writes.length, 2);
  assert.deepEqual(f.writes[1].value, view(1.6));
  f.writes[1].resolve(); await drain();
  assert.equal(f.pending, 0);
  assert.equal(f.reconciles, 0);
});

test('returning to the starting view does not write', async (t) => {
  const f = fixture(t);
  f.gesture(1.2); f.gesture(1); await f.idle();
  assert.equal(f.writes.length, 0);
  assert.equal(f.pending, 0);
});
