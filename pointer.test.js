"use strict";
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

const Pointer = vm.runInNewContext(fs.readFileSync(path.join(__dirname, "pointer.js"), "utf8") + "\nPointer;");
const plain = value => JSON.parse(JSON.stringify(value));
const SCREEN = { x: 0, y: 0, w: 2560, h: 1440 };
const RIGHT = { x: 2560, y: 0, w: 1920, h: 1080 };
const at = (x, y) => ({ x, y });

test("a still finger is not accelerated", () => {
  assert.deepEqual(plain(Pointer.accelerate(0, 0)), { x: 0, y: 0 });
});

test("a one pixel step moves by the sensitivity plus its acceleration", () => {
  assert.deepEqual(plain(Pointer.accelerate(1, 0)), { x: 1.6 * (1 + 0.06 * 1), y: 0 });
});

test("a faster step is accelerated more per pixel", () => {
  assert.deepEqual(plain(Pointer.accelerate(-3, 4)), { x: -3 * (1.6 * (1 + 0.06 * 5)), y: 4 * (1.6 * (1 + 0.06 * 5)) });
});

test("a fresh page predicts nothing before it hears the pointer", () => {
  assert.equal(Pointer.predicted(Pointer.sent(Pointer.fresh(), 5, 5)), null);
});

test("moves are numbered from one", () => {
  assert.deepEqual(plain(Pointer.sent(Pointer.sent(Pointer.fresh(), 1, 2), 3, 4)), {
    heard: null,
    pending: [{ seq: 1, dx: 1, dy: 2 }, { seq: 2, dx: 3, dy: 4 }],
    next: 3,
  });
});

test("with no move pending the prediction is the heard position", () => {
  assert.deepEqual(plain(Pointer.predicted(Pointer.heard(Pointer.fresh(), at(100, 200), 0, SCREEN))), at(100, 200));
});

test("one pending move is added to the heard position", () => {
  const model = Pointer.sent(Pointer.heard(Pointer.fresh(), at(100, 200), 0, SCREEN), 7, -3);
  assert.deepEqual(plain(Pointer.predicted(model)), at(107, 197));
});

test("many pending moves are added in order", () => {
  const model = Pointer.sent(Pointer.sent(Pointer.heard(Pointer.fresh(), at(100, 200), 0, SCREEN), 7, -3), -2, 10);
  assert.deepEqual(plain(Pointer.predicted(model)), at(105, 207));
});

test("a heard position drops the moves it already holds and keeps the later ones", () => {
  const sent = Pointer.sent(Pointer.sent(Pointer.sent(Pointer.fresh(), 1, 0), 2, 0), 4, 0);
  const model = Pointer.heard(sent, at(103, 50), 2, SCREEN);
  assert.deepEqual(plain([model.pending, Pointer.predicted(model)]), [[{ seq: 3, dx: 4, dy: 0 }], at(107, 50)]);
});

test("a heard position that holds every move leaves none pending", () => {
  const model = Pointer.heard(Pointer.sent(Pointer.fresh(), 1, 0), at(9, 9), 1, SCREEN);
  assert.deepEqual(plain([model.pending, model.next]), [[], 2]);
});

test("a prediction stops at the monitor's last pixel", () => {
  const model = Pointer.sent(Pointer.heard(Pointer.fresh(), at(2550, 1430), 0, SCREEN), 50, 50);
  assert.deepEqual(plain(Pointer.predicted(model)), at(2559, 1439));
});

test("a prediction stops at the monitor's first pixel", () => {
  const model = Pointer.sent(Pointer.heard(Pointer.fresh(), at(2570, 5), 0, RIGHT), -50, -50);
  assert.deepEqual(plain(Pointer.predicted(model)), at(2560, 0));
});

test("a move back from the edge starts from the edge, not from beyond it", () => {
  const pushed = Pointer.sent(Pointer.heard(Pointer.fresh(), at(2550, 700), 0, SCREEN), 50, 0);
  assert.deepEqual(plain(Pointer.predicted(Pointer.sent(pushed, -20, 0))), at(2539, 700));
});

test("a view that fits the monitor is centred on the pointer", () => {
  assert.deepEqual(plain(Pointer.view(at(500, 400), { w: 200, h: 100 }, SCREEN)), { x: 400, y: 350, w: 200, h: 100 });
});

test("an odd view centres with the extra pixel after the pointer", () => {
  assert.deepEqual(plain(Pointer.view(at(500, 400), { w: 201, h: 101 }, SCREEN)), { x: 400, y: 350, w: 201, h: 101 });
});

test("a view near the monitor's start stays inside it", () => {
  assert.deepEqual(plain(Pointer.view(at(2570, 5), { w: 200, h: 100 }, RIGHT)), { x: 2560, y: 0, w: 200, h: 100 });
});

test("a view near the monitor's end stays inside it", () => {
  assert.deepEqual(plain(Pointer.view(at(2555, 1435), { w: 200, h: 100 }, SCREEN)), { x: 2360, y: 1340, w: 200, h: 100 });
});

test("a view larger than the monitor shrinks to it", () => {
  assert.deepEqual(plain(Pointer.view(at(500, 400), { w: 3000, h: 1500 }, SCREEN)), { x: 0, y: 0, w: 2560, h: 1440 });
});

test("a view inside the window shows all of itself", () => {
  assert.deepEqual(
    plain(Pointer.shown({ x: 100, y: 100, w: 200, h: 100 }, { x: 0, y: 0, w: 1000, h: 800 })),
    { x: 100, y: 100, w: 200, h: 100 },
  );
});

test("a view past the window's end shows the part that overlaps", () => {
  assert.deepEqual(
    plain(Pointer.shown({ x: 900, y: 750, w: 200, h: 100 }, { x: 0, y: 0, w: 1000, h: 800 })),
    { x: 900, y: 750, w: 100, h: 50 },
  );
});

test("a view before the window's start shows the part that overlaps", () => {
  assert.deepEqual(
    plain(Pointer.shown({ x: 0, y: 0, w: 200, h: 100 }, { x: 150, y: 60, w: 1000, h: 800 })),
    { x: 150, y: 60, w: 50, h: 40 },
  );
});

test("a view that only touches the window's edge shows nothing", () => {
  assert.equal(Pointer.shown({ x: 1000, y: 0, w: 200, h: 100 }, { x: 0, y: 0, w: 1000, h: 800 }), null);
});
