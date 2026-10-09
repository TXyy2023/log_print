import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { browserReadiness } from './browser-readiness.mjs';

const report = () => ({
  iteration_only: false, complete_selected_suite: false,
  environment: { git_head: { stdout: 'test-sha' } },
  results: ['frontend', 'build'].map(name => ({ name, status: 'pass', returncode: 0 })),
});

test('runtime failures do not suppress an independently buildable browser', () => {
  const value = report();
  value.results.push({ name: 'webui-v2', status: 'fail', returncode: 1 });
  assert.equal(browserReadiness(value, 'test-sha').ready, true);
});

test('frontend/build failure, skipped build and stale commit block browser', () => {
  for (const name of ['frontend', 'build']) {
    for (const status of ['fail', 'skip', 'interrupted']) {
      const value = report();
      Object.assign(value.results.find(row => row.name === name), { status, returncode: 1 });
      assert.equal(browserReadiness(value, 'test-sha').ready, false);
    }
  }
  assert.throws(() => browserReadiness(report(), 'other-sha'), /different commit/);
  const value = report();
  value.iteration_only = true;
  assert.throws(() => browserReadiness(value), /iteration/);
  assert.throws(() => browserReadiness({ ...report(), results: [] }), /Missing/);
  assert.throws(() => browserReadiness({ ...report(), results: [...report().results, report().results[0]] }), /duplicate/);
});

test('CLI writes false and fails closed on missing, corrupt, or incomplete reports', () => {
  const directory = mkdtempSync(join(tmpdir(), 'browser-readiness-'));
  try {
    const path = join(directory, 'results.json');
    const output = join(directory, 'output');
    for (const content of [null, '{broken', '{}']) {
      if (content !== null) writeFileSync(path, content);
      writeFileSync(output, '');
      const child = spawnSync(process.execPath, ['quality/ci/browser-readiness.mjs', path], {
        encoding: 'utf8', env: { ...process.env, GITHUB_OUTPUT: output, GITHUB_SHA: 'test-sha' },
      });
      assert.equal(child.status, 1, child.stderr);
      assert.equal(readFileSync(output, 'utf8'), 'browser_ready=false\n');
    }
    writeFileSync(path, JSON.stringify(report()));
    writeFileSync(output, '');
    const child = spawnSync(process.execPath, ['quality/ci/browser-readiness.mjs', path], {
      encoding: 'utf8', env: { ...process.env, GITHUB_OUTPUT: output, GITHUB_SHA: 'test-sha' },
    });
    assert.equal(child.status, 0, child.stderr);
    assert.equal(readFileSync(output, 'utf8'), 'browser_ready=true\n');
  } finally { rmSync(directory, { recursive: true, force: true }); }
});
