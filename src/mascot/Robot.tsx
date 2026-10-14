import type { MascotState } from "./Mascot";

export type FlightDirection = "left" | "right" | "none";

/** Idle play behaviours (Mascot.tsx). */
export type Play = "wave" | "spin" | "patrol" | "stretch";

export interface FlightLook {
  flying: boolean;
  direction: FlightDirection;
}

interface RobotProps {
  state: MascotState;
  paused: boolean;
  flight: FlightLook;
  play: Play | null;
}

/**
 * Screen Buddy's robot: an original vector drawing (white shell, navy visor, glowing cyan eyes,
 * ear discs, antenna). Drawn in the 140x140 mascot window's coordinates, with room for the float
 * movement, the bank in flight, the thruster flame and the badges, so nothing is clipped.
 *
 * Every state's eyes, badges and extras are always in the DOM; `mascot.css` shows the ones for
 * the current state with a short cross-fade, so state changes are smooth.
 */
export default function Robot({ state, paused, flight, play }: RobotProps) {
  const classes = [
    "robot",
    `robot--${state}`,
    paused && "robot--paused",
    flight.flying && "robot--flying",
    flight.flying && flight.direction !== "none" && `robot--bank-${flight.direction}`,
    play && `robot--play-${play}`,
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <svg className={classes} viewBox="0 0 140 140" role="img" aria-label="Screen Buddy">
      <title>Screen Buddy</title>
      <defs>
        <linearGradient id="sb-shell" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#ffffff" />
          <stop offset="1" stopColor="#e2e8f0" />
        </linearGradient>
        <linearGradient id="sb-visor" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#1e293b" />
          <stop offset="1" stopColor="#0f172a" />
        </linearGradient>
        <radialGradient id="sb-lens" cx="0.35" cy="0.35" r="0.8">
          <stop offset="0" stopColor="#e0f2fe" stopOpacity="0.9" />
          <stop offset="1" stopColor="#38bdf8" stopOpacity="0.35" />
        </radialGradient>
        <linearGradient id="sb-flame" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#7dd3fc" stopOpacity="1" />
          <stop offset="0.55" stopColor="#38bdf8" stopOpacity="0.7" />
          <stop offset="1" stopColor="#38bdf8" stopOpacity="0" />
        </linearGradient>
        <filter id="sb-glow" x="-60%" y="-60%" width="220%" height="220%">
          <feGaussianBlur stdDeviation="1.6" result="blur" />
          <feMerge>
            <feMergeNode in="blur" />
            <feMergeNode in="SourceGraphic" />
          </feMerge>
        </filter>
      </defs>

      {/* Ground shadow: stays on the ground while the robot floats; shrinks while it flies. */}
      <g className="robot__shadow-wrap">
        <ellipse className="robot__shadow" cx="70" cy="129" rx="24" ry="3.5" />
      </g>

      <g className="robot__float">
        {/* Leans into the direction of flight. */}
        <g className="robot__bank">
          {/* Thruster flame under the body, only while flying */}
          <g className="robot__thruster">
            <path className="robot__flame" d="M59 112 Q70 150 81 112 Z" />
            <path className="robot__flame-core" d="M64.5 112 Q70 134 75.5 112 Z" />
          </g>

          {/* Antenna */}
          <g className="robot__antenna">
            <line className="robot__line" x1="70" y1="27" x2="70" y2="17" />
            <circle className="robot__spark" cx="70" cy="14" r="4.5" filter="url(#sb-glow)" />
          </g>

          {/* Body */}
          <rect className="robot__neck" x="62" y="72" width="16" height="8" rx="3" />
          <rect className="robot__shell" x="46" y="78" width="48" height="38" rx="16" />
          <circle className="robot__chest" cx="70" cy="96" r="7.5" />
          <circle className="robot__chest-light" cx="70" cy="96" r="3" filter="url(#sb-glow)" />

          {/* Arms: the right one is swapped for the magnifier arm in the "noticed" state. */}
          <g className="robot__arm">
            <rect className="robot__shell" x="35" y="83" width="10" height="24" rx="5" transform="rotate(14 40 85)" />
          </g>
          <g className="robot__arm robot__arm--right">
            <rect className="robot__shell" x="95" y="83" width="10" height="24" rx="5" transform="rotate(-14 100 85)" />
          </g>
          {/* Raised right arm for the wave, swinging from the shoulder */}
          <g className="robot__wave-arm">
            <rect className="robot__shell" x="96" y="62" width="10" height="28" rx="5" transform="rotate(25 101 88)" />
            <circle className="robot__shell" cx="112" cy="63" r="5.5" />
          </g>

          {/* Head, tilting from the neck */}
          <g className="robot__head">
            <ellipse className="robot__ear" cx="32" cy="50" rx="6" ry="11" />
            <ellipse className="robot__ear" cx="108" cy="50" rx="6" ry="11" />
            <rect className="robot__shell" x="34" y="26" width="72" height="50" rx="22" />
            <rect className="robot__visor" x="42" y="34" width="56" height="34" rx="15" />
            <path className="robot__visor-shine" d="M50 40 Q56 37 64 37" />

            {/* Eyes: one set visible at a time */}
            <g className="eyes eyes--relaxed" filter="url(#sb-glow)">
              <ellipse cx="58" cy="51" rx="4.8" ry="6.5" />
              <ellipse cx="82" cy="51" rx="4.8" ry="6.5" />
            </g>
            <g className="eyes eyes--wide" filter="url(#sb-glow)">
              <circle cx="58" cy="50" r="7" />
              <circle cx="82" cy="50" r="7" />
              <circle className="eyes__glint" cx="60.5" cy="47.5" r="2" />
              <circle className="eyes__glint" cx="84.5" cy="47.5" r="2" />
            </g>
            <g className="eyes eyes--focused" filter="url(#sb-glow)">
              <ellipse cx="58" cy="48" rx="5.5" ry="3.2" />
              <ellipse cx="82" cy="48" rx="5.5" ry="3.2" />
            </g>
            <g className="eyes eyes--happy eyes--stroke" filter="url(#sb-glow)">
              <path d="M52 53 Q58 44 64 53" />
              <path d="M76 53 Q82 44 88 53" />
            </g>
            <g className="eyes eyes--puzzled" filter="url(#sb-glow)">
              <ellipse cx="58" cy="52" rx="4.5" ry="5.5" />
              <circle cx="82" cy="48" r="3.5" />
            </g>
            <g className="eyes eyes--sleep eyes--stroke" filter="url(#sb-glow)">
              <path d="M52 51 Q58 55 64 51" />
              <path d="M76 51 Q82 55 88 51" />
            </g>
            <g className="eyes eyes--half">
              <ellipse cx="58" cy="53" rx="5" ry="2.4" />
              <ellipse cx="82" cy="53" rx="5" ry="2.4" />
            </g>

            {/* Blush for "shy" */}
            <g className="robot__blush">
              <ellipse cx="50" cy="60" rx="4" ry="2" />
              <ellipse cx="90" cy="60" rx="4" ry="2" />
            </g>

            <path className="robot__mouth" d="M65 60 Q70 64 75 60" />
          </g>

          {/* Magnifier in the raised right hand ("noticed") */}
          <g className="robot__magnifier">
            <rect className="robot__shell" x="94" y="72" width="10" height="20" rx="5" transform="rotate(-10 99 90)" />
            <line className="robot__handle" x1="92.5" y1="66" x2="97" y2="74" />
            <circle className="robot__lens" cx="85" cy="56" r="12.5" />
            <path className="robot__lens-shine" d="M78 51 Q81 47 86 47" />
          </g>
        </g>

        {/* Badges beside the head: one per situation */}
        <text className="robot__question" x="112" y="32">
          ?
        </text>
        <g className="badge badge--lock">
          <path className="badge__shackle" d="M110.5 25 v-4 a4.5 4.5 0 0 1 9 0 v4" />
          <rect className="badge__lock-body" x="107" y="24.5" width="16" height="12" rx="3" />
          <circle className="badge__keyhole" cx="115" cy="30.5" r="1.7" />
        </g>
        <g className="badge badge--no">
          <circle className="badge__no-ring" cx="115" cy="27" r="8.5" />
          <rect className="badge__no-bar" x="109.5" y="25.5" width="11" height="3" rx="1.5" />
        </g>
        <g className="badge badge--warn">
          <path className="badge__warn-triangle" d="M115 15 L125 33 L105 33 Z" />
          <rect className="badge__warn-mark" x="114" y="20.5" width="2" height="7" rx="1" />
          <circle className="badge__warn-mark" cx="115" cy="30.3" r="1.2" />
        </g>
        <g className="badge badge--plug">
          <line className="badge__plug-prong" x1="111" y1="20" x2="111" y2="15" />
          <line className="badge__plug-prong" x1="117" y1="20" x2="117" y2="15" />
          <rect className="badge__plug-body" x="107.5" y="19.5" width="13" height="9" rx="2.5" />
          <path className="badge__plug-cable" d="M114 28.5 Q114 35 121 35" />
        </g>
      </g>

      {/* "Zzz" while paused: outside the float group, so the letters drift on their own */}
      <g className="robot__zzz">
        <text className="robot__z robot__z--1" x="104" y="32">
          z
        </text>
        <text className="robot__z robot__z--2" x="111" y="24">
          z
        </text>
        <text className="robot__z robot__z--3" x="119" y="15">
          Z
        </text>
      </g>
    </svg>
  );
}
