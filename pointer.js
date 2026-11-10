"use strict";
const Pointer = (() => {
  const SENS = 1.6, ACCEL = 0.06;
  const clamp = (v, lo, hi) => Math.max(lo, Math.min(v, hi));
  function accelerate(dx, dy) {
    const f = SENS * (1 + ACCEL * Math.hypot(dx, dy));
    return { x: dx * f, y: dy * f };
  }
  const fresh = () => ({ heard: null, pending: [], next: 1 });
  function sent(model, dx, dy) {
    return { ...model, pending: [...model.pending, { seq: model.next, dx, dy }], next: model.next + 1 };
  }
  function heard(model, at, applied, monitor) {
    return { ...model, heard: { at, monitor }, pending: model.pending.filter(move => move.seq > applied) };
  }
  const inside = (p, m) => ({ x: clamp(p.x, m.x, m.x + m.w - 1), y: clamp(p.y, m.y, m.y + m.h - 1) });
  function predicted(model) {
    if (!model.heard) return null;
    const m = model.heard.monitor;
    return model.pending.reduce((p, move) => inside({ x: p.x + move.dx, y: p.y + move.dy }, m), model.heard.at);
  }
  function view(at, want, monitor) {
    const w = Math.min(want.w, monitor.w), h = Math.min(want.h, monitor.h);
    return {
      x: clamp(at.x - Math.floor(w / 2), monitor.x, monitor.x + monitor.w - w),
      y: clamp(at.y - Math.floor(h / 2), monitor.y, monitor.y + monitor.h - h),
      w,
      h,
    };
  }
  function shown(v, win) {
    const x = Math.max(v.x, win.x), y = Math.max(v.y, win.y);
    const w = Math.min(v.x + v.w, win.x + win.w) - x, h = Math.min(v.y + v.h, win.y + win.h) - y;
    return w > 0 && h > 0 ? { x, y, w, h } : null;
  }
  return Object.freeze({ accelerate, fresh, sent, heard, predicted, view, shown });
})();
