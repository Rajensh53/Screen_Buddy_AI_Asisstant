import { useEffect, useRef, type CSSProperties } from "react";
import type { MascotState } from "./Mascot";
import { PIXELS_404, faceFor } from "./face";
import { animateTurn } from "./turn";

export type FlightDirection = "left" | "right" | "none";

/** Idle play behaviours (Mascot.tsx). `greet` goes with the startup greeting. */
export type Play = "wink" | "yawn" | "glance" | "turn" | "patrol" | "greet";

/** Hologram glitches: appearing at start, vanishing on Quit, a flicker between states. */
export type Glitch = "boot" | "exit" | "jitter";

export interface FlightLook {
  flying: boolean;
  direction: FlightDirection;
}

interface RobotProps {
  state: MascotState;
  paused: boolean;
  flight: FlightLook;
  play: Play | null;
  glitch: Glitch | null;
}

/** The size of a "404" pixel, and where the digits start (centered in the visor). */
const PIXEL = 3.4;
const PIXELS_LEFT = 80 - (11 * PIXEL) / 2;
const PIXELS_TOP = 65 - (5 * PIXEL) / 2;

/** A path shape set through the CSS `d` property, so a change morphs (transition in mascot.css). */
const shape = (d: string) => ({ d, style: { d: `path("${d}")` } as CSSProperties });

/**
 * Screen Buddy: an original vector drawing of a floating robot head. A white aerodynamic shell with
 * graphite ear pods and a glossy black OLED visor, where glowing LED eyes, a mouth and icons show
 * what is going on. A cyan light cone and a glow pool underneath make it float.
 *
 * Layers (HTML, so the hover and glitches run on the GPU compositor):
 * pool (stays on the ground) / hover (floats) / bank (leans in flight) / the SVG and, only while a
 * glitch runs, its hologram copies. The "turn" play is driven frame by frame from `turn.ts`.
 * Every state's features are always in the DOM; `mascot.css` shows the ones for the current state.
 */
export default function Robot({ state, paused, flight, play, glitch }: RobotProps) {
  const classes = [
    "robot",
    `robot--${state}`,
    paused && "robot--paused",
    flight.flying && "robot--flying",
    flight.flying && flight.direction !== "none" && `robot--bank-${flight.direction}`,
    play && `robot--play-${play}`,
    glitch && `robot--glitch-${glitch}`,
  ]
    .filter(Boolean)
    .join(" ");

  const root = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (play !== "turn" || !root.current) return;
    return animateTurn(root.current);
  }, [play]);

  const face = faceFor(state, paused, play);

  return (
    <div ref={root} className={classes} role="img" aria-label="Screen Buddy">
      <div className="robot__pool" />
      <div className="robot__hover">
        <div className="robot__bank">
          <svg className="robot__svg" viewBox="0 0 160 150">
            <defs>
              <radialGradient id="sb-shell" cx="0.36" cy="0.22" r="0.95">
                <stop offset="0" stopColor="#ffffff" />
                <stop offset="0.45" stopColor="#eef1f6" />
                <stop offset="0.82" stopColor="#c4cbd7" />
                <stop offset="1" stopColor="#9aa6b8" />
              </radialGradient>
              <linearGradient id="sb-shell-under" x1="0" y1="0" x2="0" y2="1">
                <stop offset="0.6" stopColor="#38bdf8" stopOpacity="0" />
                <stop offset="1" stopColor="#2b8cff" stopOpacity="0.8" />
              </linearGradient>
              <linearGradient id="sb-pod" x1="0" y1="0" x2="1" y2="1">
                <stop offset="0" stopColor="#465165" />
                <stop offset="1" stopColor="#0f141d" />
              </linearGradient>
              <radialGradient id="sb-visor" cx="0.5" cy="0.28" r="0.85">
                <stop offset="0" stopColor="#18233a" />
                <stop offset="0.55" stopColor="#0d1322" />
                <stop offset="0.82" stopColor="#0a0e17" />
                <stop offset="1" stopColor="#05070c" />
              </radialGradient>
              <radialGradient id="sb-tint" cx="0.5" cy="0.55" r="0.65">
                <stop className="robot__led-stop" offset="0" stopOpacity="0.6" />
                <stop className="robot__led-stop" offset="1" stopOpacity="0" />
              </radialGradient>
              <linearGradient id="sb-glare" x1="0" y1="0" x2="0" y2="1">
                <stop offset="0" stopColor="#ffffff" stopOpacity="0.14" />
                <stop offset="1" stopColor="#ffffff" stopOpacity="0" />
              </linearGradient>
              <linearGradient id="sb-cone" x1="0" y1="0" x2="0" y2="1">
                <stop offset="0" stopColor="#3ee6ff" stopOpacity="0.55" />
                <stop offset="1" stopColor="#3ee6ff" stopOpacity="0" />
              </linearGradient>
              <linearGradient id="sb-sweep" x1="0" y1="0" x2="1" y2="0">
                <stop className="robot__led-stop" offset="0" stopOpacity="0" />
                <stop className="robot__led-stop" offset="0.8" stopOpacity="0.45" />
                <stop className="robot__led-stop" offset="1" stopOpacity="0" />
              </linearGradient>
              <pattern id="sb-scan" width="4" height="2.4" patternUnits="userSpaceOnUse">
                <rect width="4" height="0.9" fill="#ffffff" opacity="0.028" />
              </pattern>
              <clipPath id="sb-visor-clip">
                <rect x="36" y="36" width="88" height="58" rx="24" />
              </clipPath>
              {/* The LED glow: the shape plus a soft halo of its own color. */}
              <filter id="sb-glow" x="-50%" y="-50%" width="200%" height="200%">
                <feGaussianBlur stdDeviation="1.8" result="blur" />
                <feMerge>
                  <feMergeNode in="blur" />
                  <feMergeNode in="blur" />
                  <feMergeNode in="SourceGraphic" />
                </feMerge>
              </filter>
              <filter id="sb-soft" x="-50%" y="-50%" width="200%" height="200%">
                <feGaussianBlur stdDeviation="2.4" />
              </filter>
              <filter id="sb-haze" x="-50%" y="-50%" width="200%" height="200%">
                <feGaussianBlur stdDeviation="4.5" />
              </filter>
              {/* Single-color copies of the head for the hologram's red/cyan split. */}
              <filter id="sb-mono-red">
                <feColorMatrix values="0 0 0 0 1  0 0 0 0 0.16  0 0 0 0 0.36  0 0 0 0.8 0" />
              </filter>
              <filter id="sb-mono-cyan">
                <feColorMatrix values="0 0 0 0 0.12  0 0 0 0 0.9  0 0 0 0 1  0 0 0 0.8 0" />
              </filter>
            </defs>

            {/* The light cone under the head (moves with it) */}
            <path className="robot__cone" d="M58 100 L102 100 L124 146 L36 146 Z" filter="url(#sb-haze)" />

            <g id="sb-head" className="robot__hop">
              {/* Ear pods, behind the shell */}
              <g className="robot__ear robot__ear--left">
                <rect className="robot__pod" x="14" y="42" width="16" height="44" rx="8" />
                <rect className="robot__pod-ring" x="18" y="49" width="8" height="30" rx="4" />
              </g>
              <g className="robot__ear robot__ear--right">
                <rect className="robot__pod" x="130" y="42" width="16" height="44" rx="8" />
                <rect className="robot__pod-ring" x="134" y="49" width="8" height="30" rx="4" />
              </g>

              {/* The shell: matte white, blue from the glow underneath, a soft highlight */}
              <g className="robot__skull">
                <rect className="robot__shell" x="24" y="16" width="112" height="94" rx="46" ry="44" />
                <rect x="24" y="16" width="112" height="94" rx="46" ry="44" fill="url(#sb-shell-under)" />
                <ellipse
                  className="robot__highlight"
                  cx="58"
                  cy="29"
                  rx="20"
                  ry="6.5"
                  transform="rotate(-14 58 29)"
                  filter="url(#sb-soft)"
                />
              </g>

              {/* The back of the head: only seen while turning around */}
              <g className="robot__back-head">
                <line className="robot__vent" x1="68" y1="46" x2="92" y2="46" />
                <line className="robot__vent" x1="66" y1="54" x2="94" y2="54" />
                <line className="robot__vent" x1="68" y1="62" x2="92" y2="62" />
                <circle className="robot__port" cx="80" cy="80" r="4" />
              </g>

              {/* The visor: bezel, glossy black screen, LEDs, glass reflection */}
              <g className="robot__face">
                <rect className="robot__bezel" x="33.5" y="33.5" width="93" height="63" rx="26.5" />
                <rect x="36" y="36" width="88" height="58" rx="24" fill="url(#sb-visor)" />
                <rect className="robot__visor-tint" x="36" y="36" width="88" height="58" rx="24" fill="url(#sb-tint)" />

                <g className="led" clipPath="url(#sb-visor-clip)">
                  <g className="led__glow" filter="url(#sb-glow)">
                    <g className="led__eyes">
                      <path className="led__eye led__eye--left" {...shape(face.left)} />
                      <path className="led__eye led__eye--right" {...shape(face.right)} />
                    </g>
                    <path className="led__mouth" {...shape(face.mouth)} />

                    {/* thinking: pulsing dots, then a loading ring */}
                    <g className="led__dots">
                      <circle cx="68" cy="65" r="3.6" />
                      <circle cx="80" cy="65" r="3.6" />
                      <circle cx="92" cy="65" r="3.6" />
                    </g>
                    <circle className="led__ring" cx="80" cy="65" r="10" />

                    {/* noticed, warning: "!" */}
                    <g className="led__alert">
                      <rect x="77.3" y="49" width="5.4" height="18" rx="2.7" />
                      <circle cx="80" cy="74" r="3" />
                    </g>
                    {/* refuse: "X" */}
                    <g className="led__cross">
                      <line x1="70" y1="55" x2="90" y2="75" />
                      <line x1="90" y1="55" x2="70" y2="75" />
                    </g>
                    {/* can't read: a glitching pixel "404" */}
                    <g className="led__404">
                      {PIXELS_404.map(([x, y]) => (
                        <rect
                          key={`${x}-${y}`}
                          x={PIXELS_LEFT + x * PIXEL}
                          y={PIXELS_TOP + y * PIXEL}
                          width={PIXEL - 0.5}
                          height={PIXEL - 0.5}
                        />
                      ))}
                    </g>
                    {/* offline: an unplugged plug */}
                    <g className="led__plug">
                      <line className="led__stroke" x1="76" y1="50" x2="76" y2="56" />
                      <line className="led__stroke" x1="84" y1="50" x2="84" y2="56" />
                      <rect x="71" y="55.5" width="18" height="12" rx="3.5" />
                      <path className="led__stroke" d="M80 67.5 Q80 76 90 77" />
                    </g>
                    {/* shy: a small lock, and blushing pixels */}
                    <g className="led__lock">
                      <path className="led__stroke" d="M104.5 80 v-3 a3.5 3.5 0 0 1 7 0 v3" />
                      <rect x="102.5" y="79.5" width="11" height="8" rx="2" />
                    </g>
                    <g className="led__blush">
                      <rect x="47" y="72" width="3" height="2" />
                      <rect x="51" y="72" width="3" height="2" />
                      <rect x="83" y="72" width="3" height="2" />
                      <rect x="87" y="72" width="3" height="2" />
                    </g>
                    {/* paused: "z z" rising */}
                    <g className="led__zzz">
                      <text className="led__z led__z--1" x="104" y="52">
                        z
                      </text>
                      <text className="led__z led__z--2" x="110" y="45">
                        z
                      </text>
                    </g>
                  </g>
                  {/* noticed: a radar sweep across the screen */}
                  <rect className="led__sweep" x="22" y="36" width="20" height="58" fill="url(#sb-sweep)" />
                  {/* OLED scanlines */}
                  <rect x="36" y="36" width="88" height="58" fill="url(#sb-scan)" />
                </g>

                <rect x="38" y="38" width="84" height="22" rx="20" fill="url(#sb-glare)" />
                <path className="robot__glare" d="M45 52 Q47 42 60 40" />
              </g>

              {/* The ear pods again, in front of the shell: shown while one turns to the front */}
              <g className="robot__ear robot__ear-front robot__ear-front--left">
                <rect className="robot__pod" x="14" y="42" width="16" height="44" rx="8" />
                <rect className="robot__pod-ring" x="18" y="49" width="8" height="30" rx="4" />
              </g>
              <g className="robot__ear robot__ear-front robot__ear-front--right">
                <rect className="robot__pod" x="130" y="42" width="16" height="44" rx="8" />
                <rect className="robot__pod-ring" x="134" y="49" width="8" height="30" rx="4" />
              </g>
            </g>
          </svg>

          {glitch && (
            <>
              <div className="glitch glitch--red">
                <svg viewBox="0 0 160 150">
                  <use href="#sb-head" filter="url(#sb-mono-red)" />
                </svg>
              </div>
              <div className="glitch glitch--cyan">
                <svg viewBox="0 0 160 150">
                  <use href="#sb-head" filter="url(#sb-mono-cyan)" />
                </svg>
              </div>
              <div className="glitch__scan" />
            </>
          )}
        </div>
      </div>
      <div className="robot__beam" />
    </div>
  );
}
