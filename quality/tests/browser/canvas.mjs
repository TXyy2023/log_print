import { expect } from '../../../project/plugins/outputs/output-webui/frontend/node_modules/@playwright/test/index.mjs';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';

export async function canvasRegression(browser, url, control, read, artifact) {
  const selected = (await read()).selected;
  const made = await control('page.create', { name: 'pinch-regression', title: 'Canvas pinch regression' });
  const id = made.result.id;
  await control('page.select', { page: id });
  await control('panel.add', { page: id, title: 'Pinch fixture', left: 0, top: 0, panel_width: 320, panel_height: 220 });
  // Exercise Vue Flow's Mac branch in Linux and Windows CI as well.
  const context = await browser.newContext({
    viewport: { width: 1280, height: 900 },
    userAgent: `Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/${browser.version()} Safari/537.36`,
  });
  await context.tracing.start({ screenshots: true, snapshots: true });
  const page = await context.newPage(), peer = await context.newPage();
  const requests = [], failures = [], pageErrors = [];
  let expectedFailure = false;
  page.on('pageerror', e => pageErrors.push(String(e)));
  page.on('request', r => {
    if (r.url().endsWith('/api/control') && r.postDataJSON()?.args?.view_zoom !== undefined)
      requests.push(r.postDataJSON());
  });
  page.on('response', r => { if (!expectedFailure && r.status() >= 400) failures.push(r.status()); });
  const config = async () => (await read()).pages.find(p => p.id === id);
  const viewport = p => p.locator('.vue-flow__transformationpane').evaluate(el => {
    const m = new DOMMatrix(getComputedStyle(el).transform);
    return { view_x: Math.round(m.e), view_y: Math.round(m.f), view_zoom: Math.round(m.a * 1000) / 1000 };
  });
  const saved = async () => {
    await expect(page.locator('.status-save')).toHaveText('所有更改已保存');
    const p = await config();
    const target = { view_x: p.view_x, view_y: p.view_y, view_zoom: p.view_zoom };
    await expect.poll(() => viewport(page)).toEqual(target);
    await expect.poll(() => viewport(peer)).toEqual(target);
  };
  let cdp;
  const pinch = scaleFactor => cdp.send('Input.synthesizePinchGesture', {
    x: 700, y: 700, scaleFactor, relativeSpeed: 400, gestureSourceType: 'mouse',
  });
  const zoomRequest = route => route.request().postDataJSON()?.args?.view_zoom !== undefined;
  let release;
  try {
    await page.goto(url); await peer.goto(url);
    await saved();
    cdp = await context.newCDPSession(page);
    for (const factor of [0.6, 1.4]) {
      const n = requests.length, before = (await config()).view_zoom;
      await pinch(factor);
      await saved();
      expect(requests.length - n).toBe(1);
      const after = (await config()).view_zoom;
      if (factor < 1) expect(after).toBeLessThan(before);
      else expect(after).toBeGreaterThan(before);
    }
    // Hold a real successful response, then continue pinching before it arrives.
    let intercepted;
    const held = new Promise(resolve => { intercepted = resolve; });
    let once = true;
    await page.route('**/api/control', async route => {
      if (!once || !zoomRequest(route)) return route.continue();
      once = false;
      const response = await route.fetch();
      await new Promise(resolve => { release = resolve; intercepted(); });
      await route.fulfill({ response });
    });
    const n = requests.length;
    await pinch(0.8); await held;
    await expect(page.locator('.status-save')).toHaveText('正在保存…');
    await pinch(1.3);
    const finalView = await viewport(page);
    release(); release = undefined;
    await saved();
    expect(await viewport(page)).toEqual(finalView);
    expect(requests.length - n).toBe(2);
    await page.unroute('**/api/control');

    // Real external viewport edit wins; the stale user edit is not silently rebased.
    expectedFailure = true;
    once = true;
    await page.route('**/api/control', async route => {
      if (!once || !zoomRequest(route)) return route.continue();
      once = false;
      await control('page.set', { page: id, view_x: 31, view_y: 47, view_zoom: 0.7 });
      await route.continue();
    });
    await pinch(1.2);
    await expect(page.locator('.status-save')).toHaveText('部分更改未保存');
    await expect(page.locator('.el-message--error')).toHaveCount(1);
    await expect(page.locator('.el-message--error')).not.toContainText('其他窗口或 CLI');
    await expect.poll(() => viewport(page)).toEqual({ view_x: 31, view_y: 47, view_zoom: 0.7 });
    await page.screenshot({ path: path.join(artifact, 'canvas-external-conflict.png') });
    await page.unroute('**/api/control');
    // An unrelated successful setting must not erase the failed-save indicator.
    await page.getByRole('button', { name: '切换主题', exact: true }).click();
    await expect(page.locator('.status-save')).toHaveText('部分更改未保存');
    await pinch(1.2); await saved();
    expectedFailure = false;

    // Also detect an external edit before a queued viewport request is sent.
    let notifyBlocked;
    const blocked = new Promise(resolve => { notifyBlocked = resolve; });
    once = true;
    await page.route('**/api/control', async route => {
      const body = route.request().postDataJSON();
      if (!once || !('theme' in (body.args || {}))) return route.continue();
      once = false;
      const response = await route.fetch();
      await new Promise(resolve => { release = resolve; notifyBlocked(); });
      await route.fulfill({ response });
    });
    await page.getByRole('button', { name: '切换主题', exact: true }).click();
    await blocked;
    const beforeGuard = requests.length;
    await pinch(1.2);
    const external = await control('page.set', { page: id, view_x: 40, view_y: 50, view_zoom: 0.6 });
    await expect(page.locator('.workspace-status code')).toHaveText(`rev ${external.revision}`);
    release(); release = undefined;
    await expect(page.locator('.status-save')).toHaveText('部分更改未保存');
    expect(requests.length).toBe(beforeGuard);
    await expect.poll(() => viewport(page)).toEqual({ view_x: 40, view_y: 50, view_zoom: 0.6 });
    await page.unroute('**/api/control');
    await pinch(1.2); await saved();

    // Repeated real backend failures share one notification, with accurate status.
    expectedFailure = true;
    await page.route('**/api/control', async route => {
      const body = route.request().postDataJSON();
      if (body.method === 'page.set' && 'theme' in body.args)
        return route.continue({ postData: JSON.stringify({ ...body, args: { ...body.args, revision: 0 } }) });
      await route.continue();
    });
    for (let i = 0; i < 3; i++) {
      await page.getByRole('button', { name: '切换主题', exact: true }).click();
      await expect(page.locator('.status-save')).toHaveText('部分更改未保存');
    }
    await expect(page.locator('.el-message--error')).toHaveCount(1);
    await page.unroute('**/api/control');
    await page.getByRole('button', { name: '切换主题', exact: true }).click();
    await saved();
    expectedFailure = false;

    // Pinching at a hard limit and a pan click with no movement must settle.
    await control('page.set', { page: id, view_zoom: 2 }); await saved();
    const limitRequests = requests.length;
    await pinch(1.3); await saved();
    expect(requests.length).toBe(limitRequests);
    await page.mouse.click(700, 700, { button: 'middle' }); await saved();
    await cdp.send('Input.synthesizeScrollGesture', { x: 700, y: 700, yDistance: -80, gestureSourceType: 'mouse' });
    await saved();
    // Switching pages flushes the old page's final view without editing the new one.
    const other = (await read()).pages.find(p => p.id === selected);
    await pinch(0.8);
    const lastView = await viewport(page);
    await control('page.select', { page: selected });
    await expect(page.locator('.status-save')).toHaveText('所有更改已保存');
    await expect.poll(async () => {
      const p = await config();
      return { view_x: p.view_x, view_y: p.view_y, view_zoom: p.view_zoom };
    }).toEqual(lastView);
    expect((await read()).pages.find(p => p.id === selected)).toEqual(other);
    await control('page.select', { page: id }); await saved();
    // Toolbar zoom and reset use the same saver, followed by persisted reload.
    await page.getByRole('button', { name: '缩小画布', exact: true }).click(); await saved();
    expect((await config()).view_zoom).toBeLessThan(2);
    await page.locator('.zoom-label').click(); await saved();
    expect((await config()).view_zoom).toBe(1);
    await page.reload(); await saved();
    await page.screenshot({ path: path.join(artifact, 'canvas-pinch-fixed.png') });
    expect(failures).toEqual([]);
    expect(pageErrors).toEqual([]);
    await writeFile(path.join(artifact, 'canvas-regression.json'), JSON.stringify({ ok: true, requests, failures, pageErrors }, null, 2));
  } catch (error) {
    await page.screenshot({ path: path.join(artifact, 'canvas-failure.png') }).catch(() => {});
    throw error;
  } finally {
    release?.();
    await context.tracing.stop({ path: path.join(artifact, 'canvas-trace.zip') });
    await context.close();
    await control('page.select', { page: selected });
    await control('page.delete', { page: id });
  }
}
