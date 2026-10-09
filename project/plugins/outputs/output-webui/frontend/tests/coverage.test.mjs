import { test } from 'node:test';
import assert from 'node:assert/strict';
import { coveragePresentation as present } from '../src/coverage.ts';

const ready = { connected: true, report: { state: 'archiving' } };
const archived = () => ({
  mode: 'archive_and_memory', writer: ready, runtime_match: true,
  streams: [{ archived: { first: '1', last: '3', empty: false }, memory: { range_count: 1 } }],
});

test('live rows never imply persistence, including stale historical coverage', () => {
  for (const coverage of [undefined, archived()]) {
    const memory = present('live', { archive_enabled: false }, coverage);
    assert.equal(memory.label, '实时缓存');
    assert.equal(memory.tone, 'neutral');
    assert.match(memory.description, /未启用归档/);
    const withArchive = present('live', { archive_enabled: true, archive_writer: ready }, coverage);
    assert.equal(withArchive.label, '实时缓存');
    assert.equal(withArchive.tone, 'neutral');
    assert.match(withArchive.description, /不代表日志已提交/);
  }
});

test('unready, unavailable, failed and stopped writers cannot claim archived live data', () => {
  for (const [writer, label] of [
    [null, '归档未就绪'],
    [{ state: 'unavailable', error: 'disconnected' }, '归档不可用'],
    [{ connected: false, report: { state: 'stopped' } }, '归档已停止'],
    [{ connected: false, report: { state: 'failed', error: 'disk full' } }, '归档失败'],
  ]) {
    const result = present('live', { archive_enabled: true, archive_writer: writer });
    assert.equal(result.label, `实时缓存 · ${label}`);
    assert.equal(result.tone, 'warning');
  }
  assert.equal(present('live', { archive_enabled: true, archive_writer: { report: { state: 'failed', error: 'disk full' } } }).error, 'disk full');
});

test('history needs coverage and confirmed archive ranges', () => {
  assert.equal(present('history', { archive_enabled: true }).label, '覆盖信息未就绪');
  const memory = present('history', {}, { mode: 'memory_only', streams: [] });
  assert.equal(memory.label, '内存范围');
  assert.equal(memory.tone, 'neutral');
  assert.equal(present('history', {}, archived()).label, '归档上下文');
  for (const streams of [[], [{ archived: { empty: true } }], [{ archived: { empty: false }, uncommitted: { first: '4' } }]]) {
    const result = present('history', {}, { ...archived(), streams });
    assert.equal(result.label, '归档待提交');
    assert.equal(result.tone, 'warning');
  }
});

test('history preserves failure, stopped-prefix and incomplete-coverage warnings', () => {
  for (const [patch, label] of [
    [{ archive_error: 'database unavailable' }, '归档失败'],
    [{ writer: { report: { state: 'failed', error: 'disk full' } } }, '归档失败'],
    [{ writer: { connected: false, report: { state: 'stopped' } } }, '归档已停止'],
    [{ gap_count: 1 }, '覆盖不完整'],
    [{ runtime_match: false }, '覆盖不完整'],
    [{ mode: 'memory_only', streams: [{ uncovered_prefix: { first: '1', last: '4' } }] }, '覆盖不完整'],
  ]) {
    const result = present('history', {}, { ...archived(), ...patch });
    assert.equal(result.label, label);
    assert.equal(result.tone, 'warning');
  }
});
