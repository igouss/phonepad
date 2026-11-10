"use strict";
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

const Pointer = vm.runInNewContext(fs.readFileSync(path.join(__dirname, "pointer.js"), "utf8") + "\nPointer;");
const plain = value => JSON.parse(JSON.stringify(value));

test("a still finger is not accelerated", () => {
  assert.deepEqual(plain(Pointer.accelerate(0, 0)), { x: 0, y: 0 });
});

test("a one pixel step moves by the sensitivity plus its acceleration", () => {
  assert.deepEqual(plain(Pointer.accelerate(1, 0)), { x: 1.6 * (1 + 0.06 * 1), y: 0 });
});

test("a faster step is accelerated more per pixel", () => {
  assert.deepEqual(plain(Pointer.accelerate(-3, 4)), { x: -3 * (1.6 * (1 + 0.06 * 5)), y: 4 * (1.6 * (1 + 0.06 * 5)) });
});

test("a view that fits the window is centred on the pointer", () => {
  assert.deepEqual(
    plain(Pointer.place({ x: 500, y: 400 }, { w: 200, h: 100 }, { x: 0, y: 0, w: 1000, h: 800 })),
    { x: 400, y: 350, w: 200, h: 100 },
  );
});

test("a view near the window's start stays inside it", () => {
  assert.deepEqual(
    plain(Pointer.place({ x: 10, y: 5 }, { w: 200, h: 100 }, { x: 0, y: 0, w: 1000, h: 800 })),
    { x: 0, y: 0, w: 200, h: 100 },
  );
});

test("a view near the window's end stays inside it", () => {
  assert.deepEqual(
    plain(Pointer.place({ x: 2990, y: 1795 }, { w: 200, h: 100 }, { x: 2000, y: 1000, w: 1000, h: 800 })),
    { x: 2800, y: 1700, w: 200, h: 100 },
  );
});

test("a view larger than the window shrinks to it", () => {
  assert.deepEqual(
    plain(Pointer.place({ x: 500, y: 400 }, { w: 1200, h: 900 }, { x: 0, y: 0, w: 1000, h: 800 })),
    { x: 0, y: 0, w: 1000, h: 800 },
  );
});

test("an odd view width centres with the extra pixel after the pointer", () => {
  assert.deepEqual(
    plain(Pointer.place({ x: 500, y: 400 }, { w: 201, h: 101 }, { x: 0, y: 0, w: 1000, h: 800 })),
    { x: 400, y: 350, w: 201, h: 101 },
  );
});
