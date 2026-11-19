/**
 * Pure, zero-DOM-dependency magnetic field & damped spring physics engine
 * for the Apple macOS 26 Tahoe / visionOS Liquid Glass radial wheel.
 */

export const MAGNETIC_RADIUS = 158;
export const MAGNETIC_MAX_PULL = 9.5;
export const SPRING_STIFFNESS = 340;
export const SPRING_DAMPING = 26;
export const REST_SCALE = 1.0;
export const PROX_MAX_SCALE = 1.09;
export const ACTIVE_SCALE = 1.18;

const SETTLE_POS_EPS = 0.002;
const SETTLE_VEL_EPS = 0.01;
const TETHER_INNER_DEADZONE = 20;
const TETHER_FULL_RADIUS = 100;
const TETHER_MAX_OPACITY = 0.95;
const TETHER_OPACITY_RATE = 18;
const TETHER_ANGLE_RATE = 24;

export interface BubbleCenter {
  cx: number;
  cy: number;
}

export interface MagneticTarget {
  prox: number;
  targetTx: number;
  targetTy: number;
  targetScale: number;
  lightX: number;
  lightY: number;
  lightAngleDeg: number;
}

export interface BubbleSpringState {
  tx: number;
  vtx: number;
  ty: number;
  vty: number;
  scale: number;
  vscale: number;
  targetTx: number;
  targetTy: number;
  targetScale: number;
  prox: number;
  lightX: number;
  lightY: number;
  lightAngleDeg: number;
}

export interface BubbleSpringFrame {
  tx: number;
  ty: number;
  scale: number;
  prox: number;
  lightX: number;
  lightY: number;
  lightAngleDeg: number;
}

export interface HubTetherState {
  angleRad: number;
  targetAngleRad: number;
  opacity: number;
  targetOpacity: number;
}

export interface HubTetherFrame {
  angleDeg: number;
  opacity: number;
}

export interface WheelSpringStepResult {
  bubbles: BubbleSpringFrame[];
  tether: HubTetherFrame;
  settled: boolean;
}

export interface WheelSpringField {
  setPointer(x: number | null, y: number | null, activeIndex: number): void;
  step(dtSec: number): WheelSpringStepResult;
  snapToRest(): void;
}

function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}

function smoothstep01(t: number): number {
  const c = clamp(t, 0, 1);
  return c * c * (3 - 2 * c);
}

function wrapAngleRad(rad: number): number {
  const twoPi = Math.PI * 2;
  return ((((rad + Math.PI) % twoPi) + twoPi) % twoPi) - Math.PI;
}

function shortestAngleDiffRad(fromRad: number, toRad: number): number {
  return wrapAngleRad(toRad - fromRad);
}

function restingMagneticTarget(isActive: boolean): MagneticTarget {
  return {
    prox: 0,
    targetTx: 0,
    targetTy: 0,
    targetScale: isActive ? ACTIVE_SCALE : REST_SCALE,
    lightX: 30,
    lightY: 20,
    lightAngleDeg: -45,
  };
}

/**
 * Computes magnetic proximity, elastic sub-center pull, target scale,
 * and cursor-facing specular highlight coordinates for a single glass puck.
 */
export function computeMagneticTarget(
  cursorX: number,
  cursorY: number,
  bubbleCx: number,
  bubbleCy: number,
  isActive: boolean,
): MagneticTarget {
  if (
    !Number.isFinite(cursorX) ||
    !Number.isFinite(cursorY) ||
    !Number.isFinite(bubbleCx) ||
    !Number.isFinite(bubbleCy)
  ) {
    return restingMagneticTarget(isActive);
  }

  const dx = cursorX - bubbleCx;
  const dy = cursorY - bubbleCy;
  const dist = Math.hypot(dx, dy);
  const prox = smoothstep01(1 - dist / MAGNETIC_RADIUS);
  const targetScale = isActive
    ? ACTIVE_SCALE
    : REST_SCALE + prox * (PROX_MAX_SCALE - REST_SCALE);

  let targetTx = 0;
  let targetTy = 0;
  if (dist > 0.001) {
    const ux = dx / dist;
    const uy = dy / dist;
    // Soft tanh saturation ensures pull < dist inside the bubble so sub-center tracking never snaps.
    const fieldWeight = isActive ? Math.sqrt(prox) : prox;
    const pullMag = MAGNETIC_MAX_PULL * Math.tanh(dist / 20) * fieldWeight;
    targetTx = ux * pullMag;
    targetTy = uy * pullMag;
  }

  let lightX = 30;
  let lightY = 20;
  let lightAngleDeg = -45;
  if (dist >= 1) {
    const ux = dx / dist;
    const uy = dy / dist;
    lightX = clamp(50 + ux * 42, 6, 94);
    lightY = clamp(50 + uy * 42, 6, 94);
    lightAngleDeg = (Math.atan2(dy, dx) * 180) / Math.PI;
  }

  return {
    prox,
    targetTx,
    targetTy,
    targetScale,
    lightX,
    lightY,
    lightAngleDeg,
  };
}

function stepScalarSpring(
  val: number,
  vel: number,
  target: number,
  dt: number,
): { val: number; vel: number; settled: boolean } {
  const acc = SPRING_STIFFNESS * (target - val) - SPRING_DAMPING * vel;
  let nextVel = vel + acc * dt;
  let nextVal = val + nextVel * dt;
  const settled =
    Math.abs(target - nextVal) < SETTLE_POS_EPS &&
    Math.abs(nextVel) < SETTLE_VEL_EPS;

  if (settled) {
    nextVal = target;
    nextVel = 0;
  }

  return { val: nextVal, vel: nextVel, settled };
}

function createInitialBubbleState(): BubbleSpringState {
  return {
    tx: 0,
    vtx: 0,
    ty: 0,
    vty: 0,
    scale: REST_SCALE,
    vscale: 0,
    targetTx: 0,
    targetTy: 0,
    targetScale: REST_SCALE,
    prox: 0,
    lightX: 30,
    lightY: 20,
    lightAngleDeg: -45,
  };
}

function applyTargetToBubble(bubble: BubbleSpringState, target: MagneticTarget): void {
  bubble.targetTx = target.targetTx;
  bubble.targetTy = target.targetTy;
  bubble.targetScale = target.targetScale;
  bubble.prox = target.prox;
  bubble.lightX = target.lightX;
  bubble.lightY = target.lightY;
  bubble.lightAngleDeg = target.lightAngleDeg;
}

/**
 * Creates a damped spring field governing all bubble pucks and the center hub's
 * directional magnetic tether arc.
 */
export function createWheelSpringField(
  centers: ReadonlyArray<BubbleCenter>,
  hubCenter: BubbleCenter = { cx: 220, cy: 220 },
): WheelSpringField {
  const safeHubCx = Number.isFinite(hubCenter.cx) ? hubCenter.cx : 220;
  const safeHubCy = Number.isFinite(hubCenter.cy) ? hubCenter.cy : 220;
  const bubbleCenters: BubbleCenter[] = centers.map((c) => ({
    cx: Number.isFinite(c.cx) ? c.cx : safeHubCx,
    cy: Number.isFinite(c.cy) ? c.cy : safeHubCy,
  }));
  const bubbles: BubbleSpringState[] = bubbleCenters.map(() =>
    createInitialBubbleState(),
  );
  const tether: HubTetherState = {
    angleRad: 0,
    targetAngleRad: 0,
    opacity: 0,
    targetOpacity: 0,
  };

  function setPointer(
    x: number | null,
    y: number | null,
    activeIndex: number,
  ): void {
    if (x === null || y === null || !Number.isFinite(x) || !Number.isFinite(y)) {
      for (let i = 0; i < bubbles.length; i += 1) {
        applyTargetToBubble(bubbles[i], restingMagneticTarget(i === activeIndex));
      }
      // Preserve last angle while fading out so the tether does not spin back to 0deg.
      tether.targetOpacity = 0;
      return;
    }

    for (let i = 0; i < bubbles.length; i += 1) {
      const c = bubbleCenters[i];
      const target = computeMagneticTarget(
        x,
        y,
        c.cx,
        c.cy,
        i === activeIndex,
      );
      applyTargetToBubble(bubbles[i], target);
    }

    const dx = x - safeHubCx;
    const dy = y - safeHubCy;
    const dist = Math.hypot(dx, dy);
    const ramp = smoothstep01(
      (dist - TETHER_INNER_DEADZONE) / (TETHER_FULL_RADIUS - TETHER_INNER_DEADZONE),
    );
    tether.targetOpacity = ramp * TETHER_MAX_OPACITY;

    if (dist > 1) {
      // 0deg points to 12 o'clock to match CSS border-top arc orientation.
      const nextAngleRad = wrapAngleRad(Math.atan2(dy, dx) + Math.PI / 2);
      tether.targetAngleRad = nextAngleRad;
      // Snap angle before becoming visible so the arc fades in already facing the cursor.
      if (tether.opacity <= 0.005) {
        tether.angleRad = nextAngleRad;
      }
    }
  }

  function step(dtSec: number): WheelSpringStepResult {
    const dt = Math.min(
      0.032,
      Math.max(0.001, Number.isFinite(dtSec) ? dtSec : 0.016),
    );
    let allSettled = true;

    for (let i = 0; i < bubbles.length; i += 1) {
      const b = bubbles[i];
      const sx = stepScalarSpring(b.tx, b.vtx, b.targetTx, dt);
      const sy = stepScalarSpring(b.ty, b.vty, b.targetTy, dt);
      const ss = stepScalarSpring(b.scale, b.vscale, b.targetScale, dt);

      b.tx = sx.val;
      b.vtx = sx.vel;
      b.ty = sy.val;
      b.vty = sy.vel;
      b.scale = ss.val;
      b.vscale = ss.vel;

      if (!sx.settled || !sy.settled || !ss.settled) {
        allSettled = false;
      }
    }

    const opacityAlpha = 1 - Math.exp(-TETHER_OPACITY_RATE * dt);
    tether.opacity += (tether.targetOpacity - tether.opacity) * opacityAlpha;
    const opacitySettled =
      Math.abs(tether.targetOpacity - tether.opacity) < SETTLE_POS_EPS;
    if (opacitySettled) {
      tether.opacity = tether.targetOpacity;
    }

    const angleAlpha = 1 - Math.exp(-TETHER_ANGLE_RATE * dt);
    const angleDiff = shortestAngleDiffRad(tether.angleRad, tether.targetAngleRad);
    if (tether.targetOpacity === 0 && opacitySettled) {
      tether.targetAngleRad = tether.angleRad;
    } else {
      tether.angleRad = wrapAngleRad(tether.angleRad + angleDiff * angleAlpha);
    }
    const remainingAngleDiff = Math.abs(
      shortestAngleDiffRad(tether.angleRad, tether.targetAngleRad),
    );
    const angleSettled =
      (tether.targetOpacity === 0 && opacitySettled) ||
      remainingAngleDiff < SETTLE_POS_EPS;
    if (angleSettled && tether.targetOpacity > 0) {
      tether.angleRad = tether.targetAngleRad;
    }

    if (!opacitySettled || !angleSettled) {
      allSettled = false;
    }

    return {
      bubbles: bubbles.map((b) => ({
        tx: b.tx,
        ty: b.ty,
        scale: b.scale,
        prox: b.prox,
        lightX: b.lightX,
        lightY: b.lightY,
        lightAngleDeg: b.lightAngleDeg,
      })),
      tether: {
        angleDeg: (tether.angleRad * 180) / Math.PI,
        opacity: tether.opacity,
      },
      settled: allSettled,
    };
  }

  function snapToRest(): void {
    const rest = restingMagneticTarget(false);
    for (let i = 0; i < bubbles.length; i += 1) {
      const b = bubbles[i];
      b.tx = 0;
      b.vtx = 0;
      b.ty = 0;
      b.vty = 0;
      b.scale = REST_SCALE;
      b.vscale = 0;
      applyTargetToBubble(b, rest);
    }
    tether.angleRad = 0;
    tether.targetAngleRad = 0;
    tether.opacity = 0;
    tether.targetOpacity = 0;
  }

  return {
    setPointer,
    step,
    snapToRest,
  };
}
