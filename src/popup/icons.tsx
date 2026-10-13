// Small inline icons for the popup's buttons: crisp at any display scale, coloured by the
// button's text colour (currentColor).

const base = {
  width: 16,
  height: 16,
  viewBox: "0 0 16 16",
  fill: "none",
  stroke: "currentColor",
  strokeWidth: 1.6,
  strokeLinecap: "round" as const,
  strokeLinejoin: "round" as const,
  "aria-hidden": true,
};

export function CopyIcon() {
  return (
    <svg {...base}>
      <rect x="5.5" y="5.5" width="8" height="8" rx="1.8" />
      <path d="M10.5 5.5V3.8a1.3 1.3 0 0 0-1.3-1.3H3.8a1.3 1.3 0 0 0-1.3 1.3v5.4a1.3 1.3 0 0 0 1.3 1.3h1.7" />
    </svg>
  );
}

export function CheckIcon() {
  return (
    <svg {...base}>
      <path d="M3 8.5l3.2 3L13 4.5" />
    </svg>
  );
}

export function RetryIcon() {
  return (
    <svg {...base}>
      <path d="M13 8a5 5 0 1 1-1.6-3.7" />
      <path d="M13 2.5v3h-3" />
    </svg>
  );
}

export function CloseIcon() {
  return (
    <svg {...base}>
      <path d="M4 4l8 8M12 4l-8 8" />
    </svg>
  );
}
