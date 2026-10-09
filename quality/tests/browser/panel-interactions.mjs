import { expect } from '../../../project/plugins/outputs/output-webui/frontend/node_modules/@playwright/test/index.mjs';
import { writeFile } from 'node:fs/promises';
import path from 'node:path';

export async function panelInteractions(browser, url, control, read, artifact) {
  const selected = (await read()).selected;
  const id = (await control('page.create', { name: 'panel-interactions', sidebar_open: false })).result.id;
  await control('page.select', { page: id });
  const log = (await control('panel.add', { page: id, title: 'Editable logs', left: 0, top: 0, panel_width: 420, panel_height: 340, columns: ['time', 'text'] })).result.id;
  const curve = (await control('panel.add', { page: id, title: 'Adjacent curve', kind: 'curve', left: 500, top: 0, panel_width: 360, panel_height: 340 })).result.id;
  await control('series.add', { page: id, panel: curve, name: 'Temperature', regex: 'temperature=(?P<value>[0-9.]+)' });
  const context = await browser.newContext({ viewport: { width: 1600, height: 1050 } });
  await context.tracing.start({ screenshots: true, snapshots: true });
  const page = await context.newPage();
  const errors = [], patches = [], checks = [];
  page.on('pageerror', e => errors.push(String(e)));
  page.on('request', r => { const body = r.url().endsWith('/api/control') ? r.postDataJSON() : null; if (body?.method === 'panel.set') patches.push(body.args); });
  const config = async () => (await read()).pages.find(p => p.id === id);
  const panel = async pid => (await config()).panels.find(p => p.id === pid);
  const node = pid => page.locator(`.panel[data-panel-id="${pid}"]`);
  const inspector = page.locator('.inspector');
  const title = () => inspector.locator('.el-form-item').filter({ hasText: '名称' }).locator('input');
  const apply = () => inspector.getByRole('button', { name: '应用', exact: true });
  const synced = async () => {
    const revision = (await read()).revision;
    await expect(page.locator('.workspace-status code')).toHaveText(`rev ${revision}`);
    await expect(page.locator('.status-save')).toHaveText('所有更改已保存');
  };
  const select = async pid => { await node(pid).getByRole('button', { name: '面板属性', exact: true }).click(); await synced(); };
  const pinch = async target => {
    const box = await target.boundingBox();
    const before = (await config()).view_zoom;
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.keyboard.down('Control'); await page.mouse.wheel(0, -24); await page.keyboard.up('Control');
    await expect.poll(async () => (await config()).view_zoom).toBeGreaterThan(before);
    await synced();
  };
  try {
    await page.goto(url); await synced();
    await expect(node(log).locator('.ag-row').first()).toBeVisible();
    await select(log);
    await title().fill('Applied after pinch');
    await pinch(node(log).locator('.log-grid'));
    const chartBefore = await panel(curve);
    await pinch(node(curve).locator('.chart canvas'));
    expect((await panel(curve)).zoom_start).toEqual(chartBefore.zoom_start);
    expect((await panel(curve)).zoom_end).toEqual(chartBefore.zoom_end);
    await apply().click();
    await expect.poll(async () => (await panel(log)).title).toBe('Applied after pinch'); await synced();
    const firstPatch = patches.find(p => p.title === 'Applied after pinch');
    expect(Object.keys(firstPatch).sort()).toEqual(['page', 'panel', 'revision', 'title']);
    await inspector.getByText('十六进制', { exact: true }).click(); await apply().click();
    await expect.poll(async () => (await panel(log)).format).toBe('hex'); await synced();
    await expect(node(log).locator('.ag-row').last()).not.toContainText('temperature=');
    await inspector.getByText('文本', { exact: true }).click(); await apply().click(); await synced();
    checks.push('pinch over logs and charts; field patch survives viewport revisions; hex format applies');

    await page.locator('.zoom-label').click(); await synced();
    await title().fill('Draft survives selection');
    await select(curve); await select(log);
    await expect(title()).toHaveValue('Draft survives selection');
    await page.getByRole('button', { name: '切换属性面板', exact: true }).click(); await synced();
    await page.getByRole('button', { name: '切换属性面板', exact: true }).click(); await synced();
    await expect(title()).toHaveValue('Draft survives selection');
    await apply().click(); await expect.poll(async () => (await panel(log)).title).toBe('Draft survives selection'); await synced();
    checks.push('draft survives panel switching and inspector close/reopen');

    // Race the guarded patch at the server, beyond the client's current state.
    await title().fill('Retry after unrelated update');
    let raced = false;
    await page.route('**/api/control', async route => {
      const body = route.request().postDataJSON();
      if (!raced && body.method === 'panel.set' && body.args.title === 'Retry after unrelated update') {
        raced = true; await control('panel.set', { page: id, panel: curve, title: 'Remote curve' });
      }
      await route.continue();
    });
    await apply().click();
    await expect.poll(async () => (await panel(log)).title).toBe('Retry after unrelated update');
    expect((await panel(curve)).title).toBe('Remote curve'); await synced(); await page.unroute('**/api/control');
    checks.push('bounded server revision retry preserves unrelated edits');

    await title().fill('My title');
    await control('panel.set', { page: id, panel: log, title: 'Their title' });
    await expect(inspector.locator('.field-conflict')).toContainText('名称已被修改');
    await expect(apply()).toBeDisabled();
    await inspector.getByRole('button', { name: '保留我的修改', exact: true }).click();
    await apply().click(); await expect.poll(async () => (await panel(log)).title).toBe('My title'); await synced();
    checks.push('same-field conflict requires a scoped explicit choice');

    await title().fill('Retry after connection failure');
    await page.route('**/api/control', route => route.request().postDataJSON().method === 'panel.set' ? route.abort('failed') : route.continue());
    await apply().click(); await expect(inspector.locator('[role="alert"]')).toBeVisible();
    await expect(title()).toHaveValue('Retry after connection failure');
    await page.unroute('**/api/control'); await apply().click();
    await expect.poll(async () => (await panel(log)).title).toBe('Retry after connection failure'); await synced();
    checks.push('failed save retains draft and succeeds on explicit retry');

    await page.getByRole('button', { name: '切换属性面板', exact: true }).click(); await synced();
    await control('page.set', { page: id, view_zoom: 1, view_x: 24, view_y: 24 }); await synced();
    // Normal wheel scroll stays in the table and does not pan the canvas.
    const beforeScroll = await config();
    const body = await node(log).locator('.log-grid').boundingBox();
    await page.mouse.move(body.x + body.width / 2, body.y + body.height / 2); await page.mouse.wheel(0, -200);
    await page.waitForTimeout(250);
    expect((await config()).view_y).toBe(beforeScroll.view_y);
    await page.getByRole('button', { name: '平移工具', exact: true }).click(); await synced();
    await page.mouse.move(body.x + body.width / 2, body.y + body.height / 2); await page.mouse.wheel(0, 40);
    await expect.poll(async () => (await config()).view_y).not.toBe(beforeScroll.view_y); await synced();
    await page.getByRole('button', { name: '选择工具', exact: true }).click(); await synced();
    await page.locator('.zoom-label').click(); await synced();
    // Drag near the next panel: exact eight-pixel gutter, never overlap.
    const header = await node(log).locator('.panel-drag').boundingBox();
    await page.mouse.move(header.x + 40, header.y + 14); await page.mouse.down();
    await page.mouse.move(header.x + 111, header.y + 17, { steps: 10 });
    await expect(page.locator('.layout-preview')).toBeVisible();
    await page.mouse.up();
    await expect.poll(async () => (await panel(log)).left).toBe(72); await synced();
    expect((await panel(log)).top).toBe(0);
    const header2 = await node(log).locator('.panel-drag').boundingBox();
    await page.mouse.move(header2.x + 40, header2.y + 14); await page.mouse.down();
    await page.mouse.move(header2.x + 160, header2.y + 14, { steps: 10 }); await page.mouse.up(); await synced();
    const a = await panel(log), b = await panel(curve);
    expect(a.left + a.panel_width + 8 <= b.left || b.left + b.panel_width + 8 <= a.left || a.top + a.panel_height + 8 <= b.top || b.top + b.panel_height + 8 <= a.top).toBeTruthy();
    checks.push('table scroll, eight-pixel edge snap and collision avoidance');

    await page.getByRole('button', { name: '工作台菜单', exact: true }).click();
    await page.getByRole('menuitem', { name: '紧凑排列面板', exact: true }).click(); await synced();
    const compacted = await config();
    expect(compacted.panels[1].left).toBe(compacted.panels[0].left + compacted.panels[0].panel_width + 8);
    await page.screenshot({ path: path.join(artifact, 'panels-tiled.png') });
    checks.push('compact arrangement');
    // New panels and copies choose unoccupied slots without changing existing panels.
    await page.getByRole('button', { name: '添加面板', exact: true }).click();
    await page.getByRole('menuitem', { name: '日志监视器', exact: true }).click();
    await expect.poll(async () => (await config()).panels.length).toBe(3); await synced();
    const newPanel = (await config()).panels.find(p => p.id !== log && p.id !== curve);
    const separate = (a, b) => a.left + a.panel_width + 8 <= b.left || b.left + b.panel_width + 8 <= a.left || a.top + a.panel_height + 8 <= b.top || b.top + b.panel_height + 8 <= a.top;
    for (const other of compacted.panels) expect(separate(newPanel, other)).toBeTruthy();
    await control('panel.remove', { page: id, panel: newPanel.id });
    await control('page.set', { page: id, inspector_open: false }); await synced();
    await node(log).getByRole('button', { name: '面板菜单', exact: true }).click();
    await page.getByRole('menuitem', { name: '复制面板', exact: true }).click();
    await expect.poll(async () => (await config()).panels.length).toBe(3); await synced();
    await expect(page.getByRole('menuitem', { name: '复制面板', exact: true })).not.toBeVisible();
    const copy = (await config()).panels.find(p => p.id !== log && p.id !== curve);
    for (const other of compacted.panels) expect(separate(copy, other)).toBeTruthy();
    await control('panel.remove', { page: id, panel: copy.id }); await synced();
    checks.push('new and cloned panels avoid existing geometry');

    await node(curve).getByRole('button', { name: '面板菜单', exact: true }).click();
    await page.getByRole('menuitem', { name: '删除面板…', exact: true }).click();
    const confirm = page.locator('.panel-remove-confirm:visible');
    await expect(confirm).toContainText('Remote curve');
    await expect(page.locator('.el-overlay:visible')).toHaveCount(0);
    const pop = await confirm.boundingBox(), anchor = await node(curve).getByRole('button', { name: '面板菜单', exact: true }).boundingBox();
    expect(Math.abs(pop.y - anchor.y)).toBeLessThan(200);
    await expect(page.getByRole('menuitem', { name: '删除面板…', exact: true })).not.toBeVisible();
    await expect(confirm).toHaveCSS('opacity', '1');
    await page.screenshot({ path: path.join(artifact, 'panel-local-confirm.png') });
    await confirm.getByRole('button', { name: '取消', exact: true }).click();
    expect(await panel(curve)).toBeTruthy();
    await node(curve).getByRole('button', { name: '面板菜单', exact: true }).click();
    await page.getByRole('menuitem', { name: '删除面板…', exact: true }).click();
    await confirm.getByRole('button', { name: '删除面板', exact: true }).click();
    await expect(node(curve)).toHaveCount(0); await synced();
    await page.reload(); await synced();
    expect((await panel(log)).title).toBe('Retry after connection failure');
    expect(await panel(curve)).toBeUndefined();
    checks.push('anchored confirmation, cancellation, deletion and persisted reload');
    expect(errors).toEqual([]);
    await writeFile(path.join(artifact, 'panel-interactions.json'), JSON.stringify({ ok: true, checks, errors }, null, 2));
  } catch (e) {
    await page.screenshot({ path: path.join(artifact, 'panel-interactions-failure.png') }).catch(() => {});
    throw e;
  } finally {
    await context.tracing.stop({ path: path.join(artifact, 'panel-interactions-trace.zip') });
    await context.close();
    await control('page.select', { page: selected });
    await control('page.delete', { page: id });
  }
}
