import type { CSSProperties } from "react";
import {
  ArrowClockwise,
  ArrowCounterClockwise,
  ArrowUp,
  CaretDown,
  CaretRight,
  CaretUp,
  ChartBar,
  Check,
  Circle,
  Copy,
  DotsThree,
  DownloadSimple,
  File,
  FilePlus,
  FloppyDisk,
  Folder,
  FolderOpen,
  GearSix,
  House,
  Info,
  Lock,
  MagnifyingGlassMinus,
  MagnifyingGlassPlus,
  Metronome,
  Microphone,
  Minus,
  MusicNotes,
  Pause,
  PencilSimple,
  Scissors,
  Clipboard,
  SpeakerSlash,
  FileArrowDown,
  FileAudio,
  Flag,
  HandTap,
  CrosshairSimple,
  PianoKeys,
  Play,
  Plug,
  PlugsConnected,
  Plus,
  Repeat,
  Rows,
  SidebarSimple,
  SkipBack,
  SlidersHorizontal,
  Stop,
  Trash,
  VinylRecord,
  Warning,
  Waveform,
  X,
  Cursor,
  Eraser,
  MagnetStraight,
  ArrowsOutLineHorizontal,
  SpeakerHigh,
} from "@phosphor-icons/react";

const glyphs = {
  play: Play,
  stop: Stop,
  record: Circle,
  loop: Repeat,
  toStart: SkipBack,
  undo: ArrowCounterClockwise,
  redo: ArrowClockwise,
  newFile: FilePlus,
  open: FolderOpen,
  save: FloppyDisk,
  settings: GearSix,
  plus: Plus,
  minus: Minus,
  close: X,
  check: Check,
  chevronDown: CaretDown,
  chevronRight: CaretRight,
  chevronUp: CaretUp,
  arrowUp: ArrowUp,
  refresh: ArrowClockwise,
  audio: Microphone,
  instrument: PianoKeys,
  midi: PlugsConnected,
  folder: Folder,
  file: File,
  project: MusicNotes,
  mixer: SlidersHorizontal,
  piano: PianoKeys,
  waveform: Waveform,
  plug: Plug,
  loops: VinylRecord,
  trash: Trash,
  duplicate: Copy,
  edit: PencilSimple,
  more: DotsThree,
  info: Info,
  warning: Warning,
  download: DownloadSimple,
  zoomIn: MagnifyingGlassPlus,
  zoomOut: MagnifyingGlassMinus,
  panelLeft: SidebarSimple,
  panelRight: SidebarSimple,
  panelBottom: Rows,
  home: House,
  lock: Lock,
  metronome: Metronome,
  tracks: ChartBar,
  pause: Pause,
  split: Scissors,
  paste: Clipboard,
  muteRegion: SpeakerSlash,
  exportAudio: FileArrowDown,
  importAudio: FileAudio,
  marker: Flag,
  tap: HandTap,
  follow: CrosshairSimple,
  pointer: Cursor,
  pencil: PencilSimple,
  eraser: Eraser,
  snap: MagnetStraight,
  legato: ArrowsOutLineHorizontal,
  audition: SpeakerHigh,
} as const;
export type IconName = keyof typeof glyphs;
interface IconProps {
  name: IconName;
  size?: number;
  className?: string;
  style?: CSSProperties;
  title?: string;
}
const filled = new Set<IconName>([
  "play",
  "pause",
  "stop",
  "record",
  "audio",
  "instrument",
  "folder",
  "mixer",
  "tracks",
]);

export function Icon({ name, size = 16, className, style, title }: IconProps) {
  const Glyph = glyphs[name];
  return (
    <Glyph
      size={size}
      weight={filled.has(name) ? "fill" : "regular"}
      className={className ? `icon ${className}` : "icon"}
      style={style}
      aria-hidden={title ? undefined : true}
      aria-label={title}
      role={title ? "img" : undefined}
      data-icon={name}
    />
  );
}
