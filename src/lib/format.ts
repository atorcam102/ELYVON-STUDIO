// Display formatting and control tapers.

/** Must match `Track::MIN_VOLUME_DB` / `MAX_VOLUME_DB` in elyvon-project. */
export const MIN_DB = -96;
export const MAX_DB = 6;

export function formatDb(db: number): string {
  if (db <= MIN_DB) return "−∞";
  const rounded = Math.round(db * 10) / 10;
  const text = Math.abs(rounded).toFixed(1);
  if (rounded > 0) return `+${text}`;
  if (rounded < 0) return `−${text}`;
  return "0.0";
}

export function formatPan(pan: number): string {
  const v = Math.round(pan * 64);
  if (v === 0) return "C";
  return v < 0 ? `L${-v}` : `R${v}`;
}

/**
 * Fader taper: position 0…1 ↔ dB. Upper 75 % of travel covers −24…+6 dB
 * linearly (fine control where mixing happens), the bottom quarter fades
 * from −24 dB to −∞ quadratically.
 */
const KNEE_POS = 0.25;
const KNEE_DB = -24;

export function faderPosToDb(pos: number): number {
  const p = Math.min(1, Math.max(0, pos));
  if (p <= 0) return MIN_DB;
  if (p >= KNEE_POS) return KNEE_DB + ((p - KNEE_POS) / (1 - KNEE_POS)) * (MAX_DB - KNEE_DB);
  const t = p / KNEE_POS;
  return MIN_DB + (KNEE_DB - MIN_DB) * Math.sqrt(t);
}

export function dbToFaderPos(db: number): number {
  if (db <= MIN_DB) return 0;
  if (db >= KNEE_DB) return KNEE_POS + ((Math.min(db, MAX_DB) - KNEE_DB) / (MAX_DB - KNEE_DB)) * (1 - KNEE_POS);
  const t = ((db - MIN_DB) / (KNEE_DB - MIN_DB)) ** 2;
  return t * KNEE_POS;
}

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(v < 10 ? 1 : 0)} ${units[i]}`;
}

export function fileName(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] ?? path;
}

export function stripProjectExtension(name: string): string {
  return name.replace(/\.elyvon$/i, "");
}
