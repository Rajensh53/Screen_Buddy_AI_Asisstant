/**
 * The LED face on the visor: the shapes of the eyes and the mouth for each state.
 *
 * Every shape is one closed path of exactly four cubic curves starting at its left edge, so the
 * browser can morph any shape into any other (the CSS `d` property with a transition, see
 * mascot.css): round eyes widen, curve into `^` arcs or close into sleepy half-moons.
 */
import type { MascotState } from "./Mascot";
import type { Play } from "./Robot";

/** Eye centers and the mouth center in the 160x150 view box. */
const LEFT_EYE = { x: 62, y: 60 };
const RIGHT_EYE = { x: 98, y: 60 };
const MOUTH = { x: 80, y: 77 };
/** Bezier handle length for a quarter ellipse. */
const K = 0.5523;

const n = (v: number) => Number(v.toFixed(2));
const path = (start: [number, number], curves: number[][]) =>
  `M${n(start[0])} ${n(start[1])}` + curves.map((c) => `C${c.map(n).join(" ")}`).join("") + "Z";

/** An oval eye (or an `O` mouth). */
export function oval(cx: number, cy: number, rx: number, ry: number): string {
  return path(
    [cx - rx, cy],
    [
      [cx - rx, cy - K * ry, cx - K * rx, cy - ry, cx, cy - ry],
      [cx + K * rx, cy - ry, cx + rx, cy - K * ry, cx + rx, cy],
      [cx + rx, cy + K * ry, cx + K * rx, cy + ry, cx, cy + ry],
      [cx - K * rx, cy + ry, cx - rx, cy + K * ry, cx - rx, cy],
    ],
  );
}

/** A happy `^` eye: an arch `w` half-wide and `h` high, `t` thick at the top. */
export function arch(cx: number, cy: number, w: number, h: number, t: number): string {
  const top = cy - h / 2;
  const foot = cy + h / 2;
  return path(
    [cx - w, foot],
    [
      [cx - w, cy - h * 0.1, cx - w * 0.55, top, cx, top],
      [cx + w * 0.55, top, cx + w, cy - h * 0.1, cx + w, foot],
      [cx + w - t, cy + h * 0.05, cx + (w - t) * 0.55, top + t, cx, top + t],
      [cx - (w - t) * 0.55, top + t, cx - w + t, cy + h * 0.05, cx - w, foot],
    ],
  );
}

/** A sleepy eye: the lower half of an oval under a nearly flat lid. */
export function halfMoon(cx: number, cy: number, rx: number, ry: number): string {
  const lid = cy - 1.2;
  return path(
    [cx - rx, cy],
    [
      [cx - rx, cy + K * ry, cx - K * rx, cy + ry, cx, cy + ry],
      [cx + K * rx, cy + ry, cx + rx, cy + K * ry, cx + rx, cy],
      [cx + rx * 0.55, lid, cx + rx * 0.25, lid, cx, lid],
      [cx - rx * 0.25, lid, cx - rx * 0.55, lid, cx - rx, cy],
    ],
  );
}

/** A smile `w` half-wide and `d` deep, `t` thick in the middle (negative `d`: a frown). */
export function smile(cx: number, cy: number, w: number, d: number, t: number): string {
  const inner = d - t;
  return path(
    [cx - w, cy],
    [
      [cx - w, cy + d * 0.55, cx - w * 0.55, cy + d, cx, cy + d],
      [cx + w * 0.55, cy + d, cx + w, cy + d * 0.55, cx + w, cy],
      [cx + w * 0.75, cy + inner * 0.5, cx + w * 0.45, cy + inner, cx, cy + inner],
      [cx - w * 0.45, cy + inner, cx - w * 0.75, cy + inner * 0.5, cx - w, cy],
    ],
  );
}

/** An open, talking mouth: a flat top and a round bottom, `h` high. */
export function openMouth(cx: number, cy: number, w: number, h: number): string {
  const top = cy - h * 0.35;
  const bottom = cy + h * 0.65;
  return path(
    [cx - w, top],
    [
      [cx - w, cy + h * 0.3, cx - w * 0.6, bottom, cx, bottom],
      [cx + w * 0.6, bottom, cx + w, cy + h * 0.3, cx + w, top],
      [cx + w * 0.5, top - 1.2, cx + w * 0.2, top - 1.2, cx, top - 1.2],
      [cx - w * 0.2, top - 1.2, cx - w * 0.5, top - 1.2, cx - w, top],
    ],
  );
}

export interface Face {
  left: string;
  right: string;
  mouth: string;
}

const eyes = (shape: (cx: number, cy: number) => string) => ({
  left: shape(LEFT_EYE.x, LEFT_EYE.y),
  right: shape(RIGHT_EYE.x, RIGHT_EYE.y),
});

const RELAXED = eyes((x, y) => oval(x, y, 6.5, 8.5));
const HAPPY = eyes((x, y) => arch(x, y + 1, 8, 8, 3));
const SMALL_SMILE = smile(MOUTH.x, MOUTH.y, 6, 3.2, 1.8);

const IDLE: Face = { ...RELAXED, mouth: SMALL_SMILE };

const FACES: Record<MascotState, Face> = {
  idle: IDLE,
  noticed: { ...eyes((x, y) => oval(x, y - 1, 8.5, 10.5)), mouth: oval(MOUTH.x, MOUTH.y + 1, 3, 3.2) },
  thinking: IDLE,
  talking: { ...HAPPY, mouth: openMouth(MOUTH.x, MOUTH.y, 7, 8) },
  // Icons replace the face in these states (mascot.css).
  cant_read: IDLE,
  refuse: IDLE,
  warning: IDLE,
  offline: IDLE,
  // Looks away and down.
  shy: {
    left: oval(LEFT_EYE.x - 6, LEFT_EYE.y + 5, 5, 6.2),
    right: oval(RIGHT_EYE.x - 6, RIGHT_EYE.y + 5, 5, 6.2),
    mouth: smile(MOUTH.x - 4, MOUTH.y + 2, 4, 2, 1.6),
  },
};

const PAUSED: Face = { ...eyes((x, y) => halfMoon(x, y + 1, 7.5, 6)), mouth: smile(MOUTH.x, MOUTH.y, 4, 1.2, 1.4) };

const PLAY_FACES: Partial<Record<Play, Face>> = {
  wink: { left: RELAXED.left, right: arch(RIGHT_EYE.x, RIGHT_EYE.y + 1, 7, 6, 2.6), mouth: smile(MOUTH.x, MOUTH.y, 7, 4, 2) },
  yawn: { ...eyes((x, y) => halfMoon(x, y + 2, 7, 2.6)), mouth: oval(MOUTH.x, MOUTH.y + 1, 5, 6) },
  greet: { left: HAPPY.left, right: arch(RIGHT_EYE.x, RIGHT_EYE.y + 1, 7, 6, 2.6), mouth: openMouth(MOUTH.x, MOUTH.y, 6, 6) },
};

/** The face for the current look. Paused wins; plays only change an idle face. */
export function faceFor(state: MascotState, paused: boolean, play: Play | null): Face {
  if (paused) return PAUSED;
  if (play && state === "idle") return PLAY_FACES[play] ?? FACES.idle;
  return FACES[state];
}

/** "404" in 3x5 pixel digits, as [x, y] cells, for the "can't read" screen. */
export const PIXELS_404: [number, number][] = (() => {
  const four = ["101", "101", "111", "001", "001"];
  const zero = ["111", "101", "101", "101", "111"];
  const cells: [number, number][] = [];
  [four, zero, four].forEach((digit, i) =>
    digit.forEach((row, y) => [...row].forEach((on, x) => on === "1" && cells.push([i * 4 + x, y]))),
  );
  return cells;
})();
