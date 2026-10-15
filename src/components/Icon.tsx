// Elyvon icon set: 16×16 grid, 1.5 px strokes, round caps and joins.
// Drawn for Elyvon; no third-party icon assets are used.

import type { CSSProperties } from "react";

const paths = {
  play: <path d="M5 3.5v9l7-4.5z" fill="currentColor" stroke="none" />,
  stop: <rect x="4" y="4" width="8" height="8" rx="1" fill="currentColor" stroke="none" />,
  record: <circle cx="8" cy="8" r="4.25" fill="currentColor" stroke="none" />,
  loop: (
    <>
      <path d="M3 7V6a2 2 0 0 1 2-2h7.5" />
      <path d="M10.5 2 12.5 4l-2 2" />
      <path d="M13 9v1a2 2 0 0 1-2 2H3.5" />
      <path d="M5.5 14 3.5 12l2-2" />
    </>
  ),
  toStart: (
    <>
      <path d="M4 3.5v9" />
      <path d="M12 4 6.5 8l5.5 4z" fill="currentColor" stroke="none" />
    </>
  ),
  undo: (
    <>
      <path d="M5.5 3.5 2.5 6.5l3 3" />
      <path d="M2.5 6.5h7a3.5 3.5 0 0 1 0 7H7" />
    </>
  ),
  redo: (
    <>
      <path d="m10.5 3.5 3 3-3 3" />
      <path d="M13.5 6.5h-7a3.5 3.5 0 0 0 0 7H9" />
    </>
  ),
  newFile: (
    <>
      <path d="M9 2H4.5a1 1 0 0 0-1 1v10a1 1 0 0 0 1 1h7a1 1 0 0 0 1-1V5.5z" />
      <path d="M9 2v3.5h3.5" />
      <path d="M8 8v4M6 10h4" />
    </>
  ),
  open: (
    <>
      <path d="M2 12.5v-8a1 1 0 0 1 1-1h3l1.5 1.5H12a1 1 0 0 1 1 1V7" />
      <path d="M2 12.5 3.8 8h11l-1.8 4.5z" />
    </>
  ),
  save: (
    <>
      <path d="M3 2.5h8l2.5 2.5v8a.5.5 0 0 1-.5.5H3a.5.5 0 0 1-.5-.5V3a.5.5 0 0 1 .5-.5z" />
      <path d="M5 2.5v3h5v-3M5 13.5V10h6v3.5" />
    </>
  ),
  settings: (
    <>
      <circle cx="8" cy="8" r="2" />
      <path d="M8 1.5v2M8 12.5v2M1.5 8h2M12.5 8h2M3.4 3.4l1.4 1.4M11.2 11.2l1.4 1.4M3.4 12.6l1.4-1.4M11.2 4.8l1.4-1.4" />
    </>
  ),
  plus: <path d="M8 3v10M3 8h10" />,
  minus: <path d="M3 8h10" />,
  close: <path d="m4 4 8 8M12 4l-8 8" />,
  check: <path d="m3 8.5 3 3 7-7" />,
  chevronDown: <path d="m4 6 4 4 4-4" />,
  chevronRight: <path d="m6 4 4 4-4 4" />,
  chevronUp: <path d="m4 10 4-4 4 4" />,
  arrowUp: <path d="M8 13V3M4 7l4-4 4 4" />,
  refresh: (
    <>
      <path d="M13 8a5 5 0 1 1-1.5-3.6" />
      <path d="M13 2.5v3h-3" />
    </>
  ),
  audio: <path d="M1.5 8h1.5M4.5 5v6M7 3v10M9.5 5.5v5M12 7v2M14.5 8h0" />,
  instrument: (
    <>
      <rect x="2" y="3" width="12" height="10" rx="1" />
      <path d="M5.5 3v10M8 3v10M10.5 3v10" />
      <path d="M4.5 3v5.5h2V3M9.5 3v5.5h2V3" fill="currentColor" stroke="none" />
    </>
  ),
  midi: (
    <>
      <circle cx="8" cy="8" r="6" />
      <circle cx="5" cy="8" r=".6" fill="currentColor" />
      <circle cx="11" cy="8" r=".6" fill="currentColor" />
      <circle cx="6" cy="5.5" r=".6" fill="currentColor" />
      <circle cx="10" cy="5.5" r=".6" fill="currentColor" />
      <circle cx="8" cy="4.6" r=".6" fill="currentColor" />
      <path d="M7 11.5h2" />
    </>
  ),
  folder: <path d="M2 12.5v-8a1 1 0 0 1 1-1h3l1.5 1.5H13a1 1 0 0 1 1 1v6.5a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1z" />,
  file: (
    <>
      <path d="M9 2H4.5a1 1 0 0 0-1 1v10a1 1 0 0 0 1 1h7a1 1 0 0 0 1-1V5.5z" />
      <path d="M9 2v3.5h3.5" />
    </>
  ),
  project: (
    <>
      <rect x="2.5" y="2.5" width="11" height="11" rx="2.5" />
      <path d="M6 5v6M6 5h4.5M6 8h3.5M6 11h4.5" />
    </>
  ),
  mixer: <path d="M4 2.5v11M8 2.5v11M12 2.5v11M2.5 10h3M6.5 5.5h3M10.5 8.5h3" />,
  piano: (
    <>
      <rect x="2" y="2.5" width="12" height="11" rx="1" />
      <path d="M2 6h12M2 9.5h12M5 2.5v11" />
      <path d="M7 4h4M8 7.5h5M7 11h3" />
    </>
  ),
  waveform: <path d="M1.5 8h2l1.5-4 2 8 2-10 2 9 1.5-3h2" />,
  plug: (
    <>
      <path d="M6 2.5v3M10 2.5v3" />
      <path d="M4 5.5h8v2a4 4 0 0 1-8 0z" />
      <path d="M8 11.5v2" />
    </>
  ),
  loops: (
    <>
      <circle cx="8" cy="8" r="5.5" />
      <path d="M8 4.5v3.5l2.5 1.5" />
    </>
  ),
  trash: (
    <>
      <path d="M2.5 4h11M6 4V2.5h4V4" />
      <path d="M4 4l.7 9a1 1 0 0 0 1 .9h4.6a1 1 0 0 0 1-.9L12 4" />
    </>
  ),
  duplicate: (
    <>
      <rect x="5" y="5" width="8.5" height="8.5" rx="1.2" />
      <path d="M11 5V3.2a.7.7 0 0 0-.7-.7H3.2a.7.7 0 0 0-.7.7v7.1a.7.7 0 0 0 .7.7H5" />
    </>
  ),
  edit: <path d="M10.5 2.5 13.5 5.5 6 13H3v-3z" />,
  more: (
    <>
      <circle cx="4" cy="8" r=".9" fill="currentColor" />
      <circle cx="8" cy="8" r=".9" fill="currentColor" />
      <circle cx="12" cy="8" r=".9" fill="currentColor" />
    </>
  ),
  info: (
    <>
      <circle cx="8" cy="8" r="6" />
      <path d="M8 7.2v4M8 4.8v.2" />
    </>
  ),
  warning: (
    <>
      <path d="M8 2.2 14.2 13H1.8z" />
      <path d="M8 6.5v3M8 11.2v.2" />
    </>
  ),
  download: (
    <>
      <path d="M8 2.5v8M4.5 7 8 10.5 11.5 7" />
      <path d="M2.5 13.5h11" />
    </>
  ),
  zoomIn: (
    <>
      <circle cx="7" cy="7" r="4.5" />
      <path d="m10.5 10.5 3 3M5 7h4M7 5v4" />
    </>
  ),
  zoomOut: (
    <>
      <circle cx="7" cy="7" r="4.5" />
      <path d="m10.5 10.5 3 3M5 7h4" />
    </>
  ),
  panelLeft: (
    <>
      <rect x="2" y="2.5" width="12" height="11" rx="1.5" />
      <path d="M6 2.5v11" />
    </>
  ),
  panelRight: (
    <>
      <rect x="2" y="2.5" width="12" height="11" rx="1.5" />
      <path d="M10 2.5v11" />
    </>
  ),
  panelBottom: (
    <>
      <rect x="2" y="2.5" width="12" height="11" rx="1.5" />
      <path d="M2 9.5h12" />
    </>
  ),
  home: <path d="M2.5 7.5 8 3l5.5 4.5V13a.5.5 0 0 1-.5.5H9.5V10h-3v3.5H3a.5.5 0 0 1-.5-.5z" />,
  lock: (
    <>
      <rect x="3.5" y="7" width="9" height="6.5" rx="1" />
      <path d="M5.5 7V5a2.5 2.5 0 0 1 5 0v2" />
    </>
  ),
  metronome: (
    <>
      <path d="M5.5 2.5h5L13 13.5H3z" />
      <path d="m8 11 3.5-6.5" />
    </>
  ),
} as const;

export type IconName = keyof typeof paths;

interface IconProps {
  name: IconName;
  size?: number;
  className?: string;
  style?: CSSProperties;
  title?: string;
}

export function Icon({ name, size = 16, className, style, title }: IconProps) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.5}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className ? `icon ${className}` : "icon"}
      style={style}
      aria-hidden={title ? undefined : true}
      role={title ? "img" : undefined}
    >
      {title && <title>{title}</title>}
      {paths[name]}
    </svg>
  );
}
