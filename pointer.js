"use strict";
const Pointer = (() => {
  const SENS = 1.6, ACCEL = 0.06;
  const clamp = (v, lo, hi) => Math.max(lo, Math.min(v, hi));
  function accelerate(dx, dy) {
    const f = SENS * (1 + ACCEL * Math.hypot(dx, dy));
    return { x: dx * f, y: dy * f };
  }
  function place(at, want, win) {
    const w = Math.min(want.w, win.w), h = Math.min(want.h, win.h);
    return {
      x: clamp(at.x - Math.floor(w / 2), win.x, win.x + win.w - w),
      y: clamp(at.y - Math.floor(h / 2), win.y, win.y + win.h - h),
      w,
      h,
    };
  }
  return Object.freeze({ accelerate, place });
})();
