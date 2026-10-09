import { appendFileSync, readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

export function browserReadiness(report, expectedSha) {
  if (!report || report.iteration_only !== false || !Array.isArray(report.results)) {
    throw new Error('Missing final full-validation report (iteration reports are not build evidence)');
  }
  if (expectedSha && report.environment?.git_head?.stdout !== expectedSha) {
    throw new Error('Validation report belongs to a different commit');
  }
  for (const name of ['frontend', 'build']) {
    const matches = report.results.filter(row => row?.name === name);
    if (matches.length !== 1) throw new Error(`Missing or duplicate ${name} result`);
    const row = matches[0];
    if (row.status !== 'pass' || row.returncode !== 0) {
      return { ready: false, reason: `${name} did not pass; browser cannot use stale build output` };
    }
  }
  return { ready: true, reason: 'Frontend and workspace build passed; run browser even if runtime suites failed' };
}

export function readReadiness(path, expectedSha) {
  return browserReadiness(JSON.parse(readFileSync(path, 'utf8')), expectedSha);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  let result;
  try {
    result = readReadiness(process.argv[2], process.env.GITHUB_SHA);
  } catch (error) {
    result = { ready: false, reason: error.message };
    process.exitCode = 1;
  }
  console.log(`Browser readiness: ${result.ready}. ${result.reason}`);
  if (process.env.GITHUB_OUTPUT) {
    appendFileSync(process.env.GITHUB_OUTPUT, `browser_ready=${result.ready}\n`);
  }
}
