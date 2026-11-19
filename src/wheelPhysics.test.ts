import { describe, expect, test } from "bun:test";
import {
  ACTIVE_SCALE,
  type BubbleCenter,
  computeMagneticTarget,
  createWheelSpringField,
  MAGNETIC_MAX_PULL,
  MAGNETIC_RADIUS,
  PROX_MAX_SCALE,
  REST_SCALE,
} from "./wheelPhysics";

const HUB_CENTER: BubbleCenter = { cx: 220, cy: 220 };
const RING_RADIUS = 124;

function createEightBubbleCenters(radius = RING_RADIUS): BubbleCenter[] {
  const centers: BubbleCenter[] = [];
  for (let i = 0; i < 8; i += 1) {
    const theta = (i * 2 * Math.PI) / 8;
    centers.push({
      cx: HUB_CENTER.cx + radius * Math.sin(theta),
      cy: HUB_CENTER.cy - radius * Math.cos(theta),
    });
  }
  return centers;
}

describe("computeMagneticTarget", () => {
  test("engages magnetic proximity on center-to-ring approach and returns zero outside MAGNETIC_RADIUS", () => {
    const topBubble = { cx: 220, cy: 220 - RING_RADIUS }; // (220, 96)

    const atCenter = computeMagneticTarget(220, 220, topBubble.cx, topBubble.cy, false);
    expect(atCenter.prox).toBeGreaterThan(0);

    // Move ~25px from center toward top bubble -> dist = 99 < MAGNETIC_RADIUS (158)
    // Cursor is below bubble center (195 > 96), so magnetic pull draws bubble downward toward cursor (targetTy > 0)
    const approaching = computeMagneticTarget(220, 195, topBubble.cx, topBubble.cy, false);
    expect(approaching.prox).toBeGreaterThan(atCenter.prox);
    expect(approaching.prox).toBeGreaterThan(0);
    expect(approaching.targetTx).toBeCloseTo(0, 6);
    expect(approaching.targetTy).toBeGreaterThan(0);

    // Cursor above top bubble (y = 76 < 96) produces upward magnetic pull (targetTy < 0)
    const aboveBubble = computeMagneticTarget(220, 76, topBubble.cx, topBubble.cy, false);
    expect(aboveBubble.prox).toBeGreaterThan(approaching.prox);
    expect(aboveBubble.targetTx).toBeCloseTo(0, 6);
    expect(aboveBubble.targetTy).toBeLessThan(0);

    // Outside MAGNETIC_RADIUS -> dist = 164 > 158
    const outside = computeMagneticTarget(220, 260, topBubble.cx, topBubble.cy, false);
    expect(outside.prox).toBe(0);
    expect(outside.targetTx).toBe(0);
    expect(outside.targetTy).toBe(0);
    expect(outside.targetScale).toBe(REST_SCALE);
  });

  test("clamps elastic magnetic pull inside the bubble, zeroes at dist=0, and handles non-finite inputs", () => {
    const bubble = { cx: 220, cy: 96 };

    // Inside bubble at dist = 15px (< 33px bubble radius): pull < dist and <= MAGNETIC_MAX_PULL
    for (const isActive of [false, true]) {
      const inside = computeMagneticTarget(232, 105, bubble.cx, bubble.cy, isActive);
      const pullMag = Math.hypot(inside.targetTx, inside.targetTy);
      expect(pullMag).toBeGreaterThan(0);
      expect(pullMag).toBeLessThan(15);
      expect(pullMag).toBeLessThanOrEqual(MAGNETIC_MAX_PULL);
    }

    // Larger offset still saturates below MAGNETIC_MAX_PULL
    const farPull = computeMagneticTarget(270, 96, bubble.cx, bubble.cy, true);
    expect(Math.hypot(farPull.targetTx, farPull.targetTy)).toBeLessThanOrEqual(MAGNETIC_MAX_PULL);

    // Exact center (dist = 0) produces zero pull and resting highlight
    const exactCenter = computeMagneticTarget(bubble.cx, bubble.cy, bubble.cx, bubble.cy, true);
    expect(exactCenter.prox).toBe(1);
    expect(exactCenter.targetTx).toBe(0);
    expect(exactCenter.targetTy).toBe(0);
    expect(exactCenter.lightX).toBe(30);
    expect(exactCenter.lightY).toBe(20);
    expect(exactCenter.lightAngleDeg).toBe(-45);

    // Non-finite inputs return neutral resting targets cleanly
    const invalidInputs: Array<[number, number, number, number, boolean]> = [
      [Number.NaN, 100, bubble.cx, bubble.cy, false],
      [220, Number.POSITIVE_INFINITY, bubble.cx, bubble.cy, false],
      [220, 100, Number.NEGATIVE_INFINITY, bubble.cy, true],
      [220, 100, bubble.cx, Number.NaN, true],
    ];
    for (const [cx, cy, bx, by, isActive] of invalidInputs) {
      const res = computeMagneticTarget(cx, cy, bx, by, isActive);
      expect(res.prox).toBe(0);
      expect(res.targetTx).toBe(0);
      expect(res.targetTy).toBe(0);
      expect(res.targetScale).toBe(isActive ? ACTIVE_SCALE : REST_SCALE);
      expect(res.lightX).toBe(30);
      expect(res.lightY).toBe(20);
      expect(res.lightAngleDeg).toBe(-45);
    }
  });

  test("scales continuously from REST_SCALE through PROX_MAX_SCALE to ACTIVE_SCALE", () => {
    const bubble = { cx: 344, cy: 220 };

    const far = computeMagneticTarget(
      bubble.cx - MAGNETIC_RADIUS - 10,
      bubble.cy,
      bubble.cx,
      bubble.cy,
      false,
    );
    expect(far.prox).toBe(0);
    expect(far.targetScale).toBe(REST_SCALE);

    const midFar = computeMagneticTarget(bubble.cx - 110, bubble.cy, bubble.cx, bubble.cy, false);
    const midNear = computeMagneticTarget(bubble.cx - 45, bubble.cy, bubble.cx, bubble.cy, false);
    expect(midFar.targetScale).toBeGreaterThan(REST_SCALE);
    expect(midFar.targetScale).toBeLessThan(midNear.targetScale);
    expect(midNear.targetScale).toBeLessThan(PROX_MAX_SCALE);

    const exact = computeMagneticTarget(bubble.cx, bubble.cy, bubble.cx, bubble.cy, false);
    expect(exact.prox).toBe(1);
    expect(exact.targetScale).toBe(PROX_MAX_SCALE);

    const active = computeMagneticTarget(bubble.cx - 110, bubble.cy, bubble.cx, bubble.cy, true);
    expect(active.targetScale).toBe(ACTIVE_SCALE);
    const activeOutside = computeMagneticTarget(10, 10, bubble.cx, bubble.cy, true);
    expect(activeOutside.targetScale).toBe(ACTIVE_SCALE);
  });

  test("positions cursor-facing specular highlight lightX, lightY, and lightAngleDeg", () => {
    const bubble = { cx: 220, cy: 96 };

    // Cursor directly above bubble
    const above = computeMagneticTarget(bubble.cx, bubble.cy - 30, bubble.cx, bubble.cy, false);
    expect(above.lightX).toBeCloseTo(50, 5);
    expect(above.lightY).toBeLessThan(50);
    expect(above.lightY).toBeGreaterThanOrEqual(6);
    expect(above.lightAngleDeg).toBeCloseTo(-90, 5);

    // Cursor directly to the right of bubble
    const right = computeMagneticTarget(bubble.cx + 40, bubble.cy, bubble.cx, bubble.cy, false);
    expect(right.lightX).toBeGreaterThan(50);
    expect(right.lightX).toBeLessThanOrEqual(94);
    expect(right.lightY).toBeCloseTo(50, 5);
    expect(right.lightAngleDeg).toBeCloseTo(0, 5);

    // Extreme diagonal stays clamped within [6, 94]
    const extreme = computeMagneticTarget(10000, -10000, bubble.cx, bubble.cy, false);
    expect(extreme.lightX).toBeGreaterThanOrEqual(6);
    expect(extreme.lightX).toBeLessThanOrEqual(94);
    expect(extreme.lightY).toBeGreaterThanOrEqual(6);
    expect(extreme.lightY).toBeLessThanOrEqual(94);
  });
});

describe("createWheelSpringField", () => {
  test("updates hub tether dead-zone, opacity ramp, directional angle, and wraps smoothly across +-180deg", () => {
    const centers = createEightBubbleCenters();
    const field = createWheelSpringField(centers, HUB_CENTER);

    // Inside dead-zone (dist <= 20): tether opacity stays 0
    field.setPointer(220, 220, -1);
    let res = field.step(0.016);
    expect(res.tether.opacity).toBe(0);

    field.setPointer(234, 234, -1); // dist ~ 19.8 <= 20
    res = field.step(0.016);
    expect(res.tether.opacity).toBe(0);

    // 12 o'clock on ring -> 0deg, opacity ramps toward 0.95
    field.setPointer(220, 96, 0);
    for (let i = 0; i < 60; i += 1) {
      res = field.step(0.016);
    }
    expect(res.tether.angleDeg).toBeCloseTo(0, 1);
    expect(res.tether.opacity).toBeCloseTo(0.95, 2);

    // 3 o'clock on ring -> 90deg, opacity ~ 0.95
    field.setPointer(344, 220, 2);
    for (let i = 0; i < 60; i += 1) {
      res = field.step(0.016);
    }
    expect(res.tether.angleDeg).toBeCloseTo(90, 1);
    expect(res.tether.opacity).toBeCloseTo(0.95, 2);

    // Smooth wraparound across +-180deg near 6 o'clock: (230, 344) -> (210, 344)
    field.setPointer(230, 344, 4);
    for (let i = 0; i < 80; i += 1) {
      res = field.step(0.016);
    }
    expect(res.tether.angleDeg).toBeGreaterThan(170);

    field.setPointer(210, 344, 4);
    for (let i = 0; i < 80; i += 1) {
      res = field.step(0.016);
      // Shortest angular path stays near +-180deg and never swings backward through 0deg
      expect(Math.abs(res.tether.angleDeg)).toBeGreaterThan(170);
    }
    expect(res.tether.angleDeg).toBeLessThan(-170);
  });

  test("converges cleanly to settled === true when stepped repeatedly with dt = 0.016", () => {
    const centers = createEightBubbleCenters();
    const field = createWheelSpringField(centers, HUB_CENTER);

    field.setPointer(336, 212, 2);
    let settledAt = -1;
    let last = field.step(0.016);
    for (let i = 1; i < 240; i += 1) {
      last = field.step(0.016);
      if (last.settled) {
        settledAt = i;
        break;
      }
    }

    expect(settledAt).toBeGreaterThan(0);
    expect(last.settled).toBe(true);
    expect(last.bubbles[2].scale).toBeCloseTo(ACTIVE_SCALE, 3);
    expect(last.bubbles[2].tx).toBeLessThan(0);
    expect(last.bubbles[2].ty).toBeLessThan(0);
    expect(last.bubbles[2].prox).toBeGreaterThan(0.95);
    expect(last.bubbles[6].scale).toBeCloseTo(REST_SCALE, 3);
    expect(last.bubbles[6].prox).toBe(0);
    expect(last.tether.opacity).toBeCloseTo(0.95, 2);

    // Clearing pointer also converges smoothly back to rest
    field.setPointer(null, null, -1);
    let restSettled = false;
    for (let i = 0; i < 240; i += 1) {
      last = field.step(0.016);
      if (last.settled) {
        restSettled = true;
        break;
      }
    }
    expect(restSettled).toBe(true);
    expect(last.tether.opacity).toBe(0);
    for (const b of last.bubbles) {
      expect(b.tx).toBe(0);
      expect(b.ty).toBe(0);
      expect(b.scale).toBe(REST_SCALE);
      expect(b.prox).toBe(0);
    }
  });

  test("clamps dtSec spikes and non-finite step intervals without NaN or divergence", () => {
    const centers = createEightBubbleCenters();
    const field = createWheelSpringField(centers, HUB_CENTER);
    field.setPointer(330, 210, 2);

    const spikeSteps = [10, Number.NaN, -5, Number.POSITIVE_INFINITY, 0, 100];
    for (const dt of spikeSteps) {
      const frame = field.step(dt);
      expect(Number.isFinite(frame.tether.angleDeg)).toBe(true);
      expect(Number.isFinite(frame.tether.opacity)).toBe(true);
      expect(frame.tether.opacity).toBeGreaterThanOrEqual(0);
      expect(frame.tether.opacity).toBeLessThanOrEqual(1);

      for (const b of frame.bubbles) {
        expect(Number.isFinite(b.tx)).toBe(true);
        expect(Number.isFinite(b.ty)).toBe(true);
        expect(Number.isFinite(b.scale)).toBe(true);
        expect(Number.isFinite(b.prox)).toBe(true);
        expect(Number.isFinite(b.lightX)).toBe(true);
        expect(Number.isFinite(b.lightY)).toBe(true);
        expect(Number.isFinite(b.lightAngleDeg)).toBe(true);
        expect(Math.hypot(b.tx, b.ty)).toBeLessThanOrEqual(MAGNETIC_MAX_PULL * 1.5);
        expect(b.scale).toBeGreaterThanOrEqual(0.8);
        expect(b.scale).toBeLessThanOrEqual(ACTIVE_SCALE + 0.15);
      }
    }
  });

  test("snapToRest zeroes all offsets, velocities, prox, and tether opacity while restoring REST_SCALE", () => {
    const centers = createEightBubbleCenters();
    const field = createWheelSpringField(centers, HUB_CENTER);
    field.setPointer(332, 210, 2);

    // Advance a few frames so positions, velocities, scale, prox, and tether are all active
    for (let i = 0; i < 5; i += 1) {
      field.step(0.016);
    }

    field.snapToRest();
    // Stepping immediately confirms velocities were also zeroed (settled on first step)
    const afterSnap = field.step(0.016);
    expect(afterSnap.settled).toBe(true);
    expect(afterSnap.tether.opacity).toBe(0);
    expect(afterSnap.tether.angleDeg).toBe(0);
    for (const b of afterSnap.bubbles) {
      expect(b.tx).toBe(0);
      expect(b.ty).toBe(0);
      expect(b.scale).toBe(REST_SCALE);
      expect(b.prox).toBe(0);
      expect(b.lightX).toBe(30);
      expect(b.lightY).toBe(20);
      expect(b.lightAngleDeg).toBe(-45);
    }
  });
});
