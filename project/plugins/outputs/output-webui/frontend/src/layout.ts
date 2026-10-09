export interface Rect { id: string; left: number; top: number; panel_width: number; panel_height: number; hidden?: boolean; locked?: boolean }
export const GAP = 8;
export function overlaps(a: Rect, b: Rect, gap = GAP): boolean {
  return a.left < b.left + b.panel_width + gap && a.left + a.panel_width + gap > b.left &&
    a.top < b.top + b.panel_height + gap && a.top + a.panel_height + gap > b.top;
}
export function available(rect: Rect, others: Rect[]): boolean {
  return others.every(p => p.id === rect.id || p.hidden || !overlaps(rect, p));
}
// Candidate edges keep placement deterministic and avoid an unbounded pixel scan.
export function place(rect: Rect, others: Rect[], origin = { x: 24, y: 24 }, width = 1400): Rect {
  const visible = others.filter(p => !p.hidden && p.id !== rect.id);
  const xs = [origin.x, ...visible.map(p => p.left + p.panel_width + GAP)].filter(x => x >= origin.x);
  const ys = [origin.y, ...visible.map(p => p.top + p.panel_height + GAP)].filter(y => y >= origin.y);
  for (const top of [...new Set(ys)].sort((a, b) => a - b)) {
    for (const left of [...new Set(xs)].sort((a, b) => a - b)) {
      if (left !== origin.x && left + rect.panel_width > origin.x + width) continue;
      const candidate = { ...rect, left, top };
      if (available(candidate, visible)) return candidate;
    }
  }
  return { ...rect, left: origin.x, top: Math.max(origin.y, ...ys) };
}
export function compact(panels: Rect[], width: number): Rect[] {
  const placed = panels.filter(p => !p.hidden && p.locked).map(p => ({ ...p }));
  for (const panel of panels.filter(p => !p.hidden && !p.locked)) placed.push(place(panel, placed, { x: 24, y: 24 }, width));
  return placed;
}
export function snapMove(rect: Rect, others: Rect[], tolerance: number, snap: boolean, allowOverlap: boolean) {
  const visible = others.filter(p => !p.hidden && p.id !== rect.id);
  const xs = visible.flatMap(p => [p.left, p.left + p.panel_width - rect.panel_width, p.left + p.panel_width + GAP, p.left - rect.panel_width - GAP]);
  const ys = visible.flatMap(p => [p.top, p.top + p.panel_height - rect.panel_height, p.top + p.panel_height + GAP, p.top - rect.panel_height - GAP]);
  const nearest = (value: number, values: number[]) => values.filter(v => Math.abs(v - value) <= tolerance).sort((a, b) => Math.abs(a - value) - Math.abs(b - value))[0];
  const sx = snap ? nearest(rect.left, xs) : undefined, sy = snap ? nearest(rect.top, ys) : undefined;
  let candidate = { ...rect, left: sx ?? rect.left, top: sy ?? rect.top };
  if (!allowOverlap && !available(candidate, visible)) {
    const candidates = [rect.left, ...xs].flatMap(left => [rect.top, ...ys].map(top => ({ ...rect, left, top })))
      .filter(p => available(p, visible))
      .sort((a, b) => Math.hypot(a.left - rect.left, a.top - rect.top) - Math.hypot(b.left - rect.left, b.top - rect.top));
    candidate = candidates[0] ?? place(rect, visible);
  }
  return candidate;
}
