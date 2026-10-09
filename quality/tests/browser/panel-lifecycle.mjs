import { expect } from '../../../project/plugins/outputs/output-webui/frontend/node_modules/@playwright/test/index.mjs';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';

// Hold a real initial response until its panel has unmounted. This made the old
// async onMounted create a polling interval after cleanup had already run.
export async function panelLifecycleRegression(browser, url, control, read, artifact) {
  const selected = (await read()).selected;
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  await context.tracing.start({ screenshots: true, snapshots: true });
  await context.addInitScript(() => {
    // Count live panel pollers as well as HTTP failures: an orphan may poll a
    // still-existing panel successfully (especially after a layout remount).
    const interval = window.setInterval.bind(window), clear = window.clearInterval.bind(window);
    window.panelPollers = new Set();
    window.setInterval = (fn, delay, ...args) => {
      const id = interval(fn, delay, ...args);
      if (delay === 750) window.panelPollers.add(id);
      return id;
    };
    window.clearInterval = id => { window.panelPollers.delete(id); clear(id); };
  });
  const cases = [], failures = [], pageErrors = [], created = new Set();
  let page, release;
  try {
    for (const [action, kind] of [
      ['switch', 'log'], ['switch', 'curve'], ['hide', 'log'],
      ['remove', 'curve'], ['delete-page', 'log'], ['layout', 'curve'],
    ]) {
      const source = (await control('page.create', { name: `lifecycle-${action}-${kind}` })).result.id;
      created.add(source);
      const old = (await control('panel.add', { page: source, kind, title: 'Delayed panel' })).result.id;
      const switches = action === 'switch' || action === 'delete-page';
      const target = switches
        ? (await control('page.create', { name: `lifecycle-target-${action}-${kind}` })).result.id
        : source;
      created.add(target);
      const healthy = action === 'layout' ? old
        : (await control('panel.add', { page: target, title: 'Active panel', left: 360 })).result.id;
      await control('page.select', { page: source });
      page = await context.newPage();
      const requests = [], responses = [];
      page.on('pageerror', error => pageErrors.push({ action, kind, error: String(error) }));
      page.on('request', request => {
        if (request.url().endsWith('/api/control')) requests.push(request.postDataJSON());
      });
      page.on('response', response => {
        const request = response.request();
        const body = request.url().endsWith('/api/control') ? request.postDataJSON() : undefined;
        responses.push({ status: response.status(), body });
        if (response.status() >= 400)
          failures.push({ action, kind, status: response.status(), url: response.url(), body });
      });
      let notifyHeld, once = true;
      const held = new Promise(resolve => { notifyHeld = resolve; });
      await page.route('**/api/control', async route => {
        const body = route.request().postDataJSON();
        if (!once || body.method !== 'panel.data' || body.args.panel !== old) return route.continue();
        once = false;
        const response = await route.fetch();
        expect(response.ok()).toBeTruthy();
        await new Promise(resolve => { release = resolve; notifyHeld(); });
        await route.fulfill({ response });
      });
      await page.goto(url);
      // Use a bounded assertion so a missing initial request fails diagnostically.
      await expect.poll(() => Boolean(release)).toBe(true);
      await held;
      const oldPanel = page.locator(`.panel[data-panel-id="${old}"]`);
      await expect(oldPanel).toBeVisible();
      if (action === 'switch') await control('page.select', { page: target });
      else if (action === 'hide') await control('panel.set', { page: source, panel: old, hidden: true });
      else if (action === 'remove') await control('panel.remove', { page: source, panel: old });
      else if (action === 'delete-page') {
        await control('page.delete', { page: source });
        created.delete(source);
        await control('page.select', { page: target });
      } else await control('page.set', { page: source, layout_mode: 'grid' });
      if (action === 'layout') await expect(page.locator('.grid-board .panel')).toHaveCount(1);
      else await expect(oldPanel).toHaveCount(0);
      await expect(page.locator(`.panel[data-panel-id="${healthy}"]`)).toBeVisible();
      release(); release = undefined;
      await page.unrouteAll({ behavior: 'wait' });
      const healthyResponses = () => responses.filter(r => r.status === 200 &&
        r.body?.method === 'panel.data' && r.body.args.page === target && r.body.args.panel === healthy).length;
      const initial = healthyResponses();
      // Observe three real polling cycles after releasing the old response.
      // The replacement must keep working while the disposed panel stays silent.
      await expect.poll(healthyResponses, { timeout: 10_000 }).toBeGreaterThanOrEqual(initial + 3);
      const pollers = await page.evaluate(() => window.panelPollers.size);
      const oldRequests = requests.filter(r => r.method === 'panel.data' && r.args.panel === old);
      cases.push({ action, kind, pollers, healthyResponses: healthyResponses(), oldRequests: oldRequests.length });
      expect(pollers).toBe(1);
      if (action !== 'layout') expect(oldRequests).toHaveLength(1);
      expect(failures).toEqual([]);
      expect(pageErrors).toEqual([]);
      await page.close(); page = undefined;
      await control('page.select', { page: selected });
      for (const id of created) await control('page.delete', { page: id });
      created.clear();
    }
  } catch (error) {
    await page?.screenshot({ path: path.join(artifact, 'panel-lifecycle-failure.png') }).catch(() => {});
    throw error;
  } finally {
    release?.();
    await page?.unrouteAll({ behavior: 'wait' });
    await writeFile(path.join(artifact, 'panel-lifecycle.json'), JSON.stringify({ cases, failures, pageErrors }, null, 2));
    await context.tracing.stop({ path: path.join(artifact, 'panel-lifecycle-trace.zip') });
    await context.close();
    await control('page.select', { page: selected });
    for (const id of created) await control('page.delete', { page: id });
  }
}
