export function initials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  if (parts.length === 0) return "?";
  const first = Array.from(parts[0])[0] ?? "?";
  const second = parts.length > 1 ? (Array.from(parts[parts.length - 1])[0] ?? "") : "";
  return (first + second).toUpperCase();
}

/** A stable hue per contact so avatars are recognisable at a glance. */
export function hueOf(id: string): number {
  let h = 0;
  for (let i = 0; i < id.length; i++) h = (h * 31 + id.charCodeAt(i)) >>> 0;
  return h % 360;
}

export function clock(ts: number): string {
  return new Date(ts * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

function startOfDay(d: Date): number {
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
}

/** Chat-list style: time today, "Yesterday", weekday this week, else a date. */
export function listTime(ts: number): string {
  const d = new Date(ts * 1000);
  const days = Math.round((startOfDay(new Date()) - startOfDay(d)) / 86_400_000);
  if (days <= 0) return clock(ts);
  if (days === 1) return "Yesterday";
  if (days < 7) return d.toLocaleDateString([], { weekday: "long" });
  return d.toLocaleDateString([], { day: "2-digit", month: "2-digit", year: "numeric" });
}

export function dayLabel(ts: number): string {
  const d = new Date(ts * 1000);
  const days = Math.round((startOfDay(new Date()) - startOfDay(d)) / 86_400_000);
  if (days <= 0) return "Today";
  if (days === 1) return "Yesterday";
  return d.toLocaleDateString([], { weekday: "long", day: "numeric", month: "long", year: "numeric" });
}

export function sameDay(a: number, b: number): boolean {
  return startOfDay(new Date(a * 1000)) === startOfDay(new Date(b * 1000));
}

export function bytes(n: number): string {
  if (n < 1000) return `${n} B`;
  return `${(n / 1000).toFixed(n < 10_000 ? 1 : 0)} kB`;
}

/** A rough passphrase score from 0 to 4 (length and variety; not a guarantee). */
export function strength(p: string): number {
  if (p.length < 8) return 0;
  const classes = [/[a-z]/, /[A-Z]/, /[0-9]/, /[^A-Za-z0-9]/].filter((r) => r.test(p)).length;
  const words = p.trim().split(/\s+/).length;
  let score = 0;
  if (p.length >= 10) score++;
  if (p.length >= 14 || words >= 4) score++;
  if (classes >= 3 || words >= 4) score++;
  if (p.length >= 20 || (words >= 5 && p.length >= 16)) score++;
  return Math.min(score, 4);
}

export function groupDigits(s: string): string[] {
  const digits = s.replace(/\D/g, "");
  return digits.match(/.{1,5}/g) ?? [];
}

export async function fileToBase64(file: File): Promise<string> {
  const buf = new Uint8Array(await file.arrayBuffer());
  let bin = "";
  for (let i = 0; i < buf.length; i += 0x8000) bin += String.fromCharCode(...buf.subarray(i, i + 0x8000));
  return btoa(bin);
}
