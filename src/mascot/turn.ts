/**
 * The idle "turn": the head turns around once on its spot, like a solid object.
 *
 * A flat squash (scaleX down to a line) looks like a paper cut-out. Instead, the head is treated as
 * an elliptic cylinder around the vertical axis at x = 80 (view-box px): its outline stays almost
 * as wide while turning, and the parts on it move across the surface. The visor slides to the side
 * and disappears behind the edge, an ear pod comes round to the front, and the back (vents) shows
 * halfway. Animation touches on top: a little wind-up and overshoot, and a crouch before the hop
 * with a squash on landing, while the glow underneath shrinks as the head rises.
 *
 * `turnPose` is pure; `animateTurn` drives the parts frame by frame.
 */

export const TURN_MS = 1700;

/** Half width and half depth of the head shell. */
const HEAD = { a: 56, b: 46 };
/** Half width of the visor and of the vents on the back. */
const VISOR_HALF = 44;
const VENTS_HALF = 14;
/** The ear pods sit this far out from the axis; seen from the front they are this wide, and
 * this wide when seen from the side (their disc). */
const EAR_X = 58;
const EAR_THICKNESS = 16;
const EAR_DISC = 30;
/** An ear pod is drawn in front of the head only once it is well round to the front. */
const EAR_FRONT = 0.35;

export type TurnPart =
  | "skull"
  | "face"
  | "backHead"
  | "earLeft"
  | "earRight"
  | "earFrontLeft"
  | "earFrontRight"
  | "hop"
  | "pool";

export interface PartPose {
  transform: string;
  visible: boolean;
}

export type TurnPose = Record<TurnPart, PartPose>;

/** The element of each part (classes in Robot.tsx). */
const SELECTORS: Record<TurnPart, string> = {
  skull: ".robot__skull",
  face: ".robot__face",
  backHead: ".robot__back-head",
  earLeft: ".robot__ear--left",
  earRight: ".robot__ear--right",
  earFrontLeft: ".robot__ear-front--left",
  earFrontRight: ".robot__ear-front--right",
  hop: ".robot__hop",
  pool: ".robot__pool",
};

const clamp01 = (t: number) => Math.min(1, Math.max(0, t));
const smooth = (t: number) => t * t * (3 - 2 * t);
const px = (n: number) => `${n.toFixed(2)}px`;
const num = (n: number) => n.toFixed(3);

/** Slow start with a little wind-up backwards, fast middle, a little overshoot, gentle stop. */
function easeInOutBack(t: number): number {
  const c = 0.9 * 1.525;
  return t < 0.5
    ? ((2 * t) ** 2 * ((c + 1) * 2 * t - c)) / 2
    : ((2 * t - 2) ** 2 * ((c + 1) * (2 * t - 2) + c) + 2) / 2;
}

/** The turn angle in degrees at `t` (0-1): still while crouching, then once around. */
export function turnAngle(t: number): number {
  const start = 0.08;
  const end = 0.92;
  return 360 * easeInOutBack(clamp01((t - start) / (end - start)));
}

/** Hop height (px, negative is up) and squash/stretch at `t`, through these key poses. */
const HOP: [t: number, y: number, sx: number, sy: number][] = [
  [0, 0, 1, 1],
  [0.1, 0, 1.05, 0.94], // crouch
  [0.22, -8, 0.97, 1.04], // take-off stretch
  [0.34, -10, 1, 1],
  [0.66, -10, 1, 1],
  [0.8, -1.5, 0.98, 1.03], // falling stretch
  [0.86, 0, 1.05, 0.95], // landing squash
  [0.94, 0, 0.99, 1.01],
  [1, 0, 1, 1],
];

function hop(t: number): { y: number; sx: number; sy: number } {
  const i = Math.max(0, HOP.findIndex(([at]) => at > t) - 1);
  const [t0, y0, sx0, sy0] = HOP[i];
  const [t1, y1, sx1, sy1] = HOP[Math.min(i + 1, HOP.length - 1)];
  const k = t1 > t0 ? smooth(clamp01((t - t0) / (t1 - t0))) : 1;
  return { y: y0 + (y1 - y0) * k, sx: sx0 + (sx1 - sx0) * k, sy: sy0 + (sy1 - sy0) * k };
}

type Ellipse = { a: number; b: number };

/** Screen x (from the axis) and depth (positive: toward the viewer) of a point on `e` at
 * `psi` (0 = the middle of the front), after turning by `theta` (radians). */
function project(e: Ellipse, psi: number, theta: number): { x: number; z: number } {
  const x = e.a * Math.sin(psi);
  const z = e.b * Math.cos(psi);
  return {
    x: x * Math.cos(theta) + z * Math.sin(theta),
    z: -x * Math.sin(theta) + z * Math.cos(theta),
  };
}

const HIDDEN: PartPose = { transform: "none", visible: false };

/** A flat part on the surface of `e`, drawn centered on the axis and `half` px to each side,
 * whose middle is at `psi` (0 front, PI back): squeezed to the part of it facing the viewer. */
function onSurface(e: Ellipse, psi: number, half: number, theta: number): PartPose {
  const spread = Math.asin(Math.min(1, half / e.a));
  let lo = Infinity;
  let hi = -Infinity;
  const steps = 16;
  for (let i = 0; i <= steps; i++) {
    const p = project(e, psi - spread + (2 * spread * i) / steps, theta);
    if (p.z > 0) {
      lo = Math.min(lo, p.x);
      hi = Math.max(hi, p.x);
    }
  }
  // Kept inside the outline: the frame around the visor would otherwise poke out at the edge.
  const edge = outline(e, theta) - 3;
  lo = Math.max(lo, -edge);
  hi = Math.min(hi, edge);
  if (hi - lo < 1.5) return HIDDEN;
  return { transform: `translateX(${px((lo + hi) / 2)}) scaleX(${num((hi - lo) / (2 * half))})`, visible: true };
}

/** The outline of `e` keeps nearly its width: wide from the front, its depth from the side. */
function outline(e: Ellipse, theta: number): number {
  return Math.hypot(e.a * Math.cos(theta), e.b * Math.sin(theta));
}

/** An ear pod at `side` (-1 left, 1 right): moves round the head; its disc shows from the side. */
function ear(side: -1 | 1, theta: number, front: boolean): PartPose {
  const x = side * EAR_X * Math.cos(theta);
  const z = -side * EAR_X * Math.sin(theta);
  const width = (EAR_THICKNESS * Math.abs(Math.cos(theta)) + EAR_DISC * Math.abs(Math.sin(theta))) / EAR_THICKNESS;
  const transform = `translateX(${px(x - side * EAR_X)}) scaleX(${num(width)})`;
  return { transform, visible: !front || z > EAR_FRONT * EAR_X };
}

/** Where every part is at `t` (0-1 of `TURN_MS`). */
export function turnPose(t: number): TurnPose {
  const theta = (turnAngle(t) * Math.PI) / 180;
  const { y, sx, sy } = hop(t);
  return {
    skull: { transform: `scaleX(${num(outline(HEAD, theta) / HEAD.a)})`, visible: true },
    face: onSurface(HEAD, 0, VISOR_HALF, theta),
    backHead: onSurface(HEAD, Math.PI, VENTS_HALF, theta),
    earLeft: ear(-1, theta, false),
    earRight: ear(1, theta, false),
    earFrontLeft: ear(-1, theta, true),
    earFrontRight: ear(1, theta, true),
    hop: { transform: `translateY(${px(y)}) scale(${num(sx)}, ${num(sy)})`, visible: true },
    pool: { transform: `scale(${num(1 + y / 25)})`, visible: true },
  };
}

/** Plays the turn on the robot inside `root`, one pose per frame. Returns a stop function, which
 * also puts every part back as the stylesheet has it. */
export function animateTurn(root: HTMLElement): () => void {
  const parts = Object.entries(SELECTORS).flatMap(([part, selector]) => {
    const el = root.querySelector<HTMLElement | SVGElement>(selector);
    return el ? [[part as TurnPart, el] as const] : [];
  });
  const start = performance.now();
  let frame = 0;
  const step = (now: number) => {
    const t = clamp01((now - start) / TURN_MS);
    const pose = turnPose(t);
    for (const [part, el] of parts) {
      el.style.transform = pose[part].transform;
      el.style.visibility = pose[part].visible ? "visible" : "hidden";
    }
    if (t < 1) frame = requestAnimationFrame(step);
  };
  frame = requestAnimationFrame(step);
  return () => {
    cancelAnimationFrame(frame);
    for (const [, el] of parts) {
      el.style.transform = "";
      el.style.visibility = "";
    }
  };
}
