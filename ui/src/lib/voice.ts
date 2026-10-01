// Voice notes: short audio clips sent as ordinary file messages named
// `voice-<unix time>-<seconds>s.<ext>`. Nothing special travels on the wire, so they
// get the same encryption, size limit and padding as every other file.
import { MAX_FILE_BYTES } from "./api";

/** Longest recording. At 24 kbit/s Opus this is about 180 kB. */
export const MAX_VOICE_SECONDS = 60;
const BITRATE = 24_000;

const FORMATS = [
  { mime: "audio/webm;codecs=opus", ext: "webm", type: "audio/webm" },
  { mime: "audio/ogg;codecs=opus", ext: "ogg", type: "audio/ogg" },
  { mime: "audio/mp4", ext: "m4a", type: "audio/mp4" },
] as const;

const NAME = /^voice-(\d{9,13})-(\d{1,3})s\.(webm|ogg|m4a)$/;

export function voiceSupported(): boolean {
  return typeof MediaRecorder !== "undefined" && !!navigator.mediaDevices?.getUserMedia && pickFormat() !== null;
}

function pickFormat() {
  return FORMATS.find((f) => MediaRecorder.isTypeSupported(f.mime)) ?? null;
}

/** Recognise a voice note by its name. Anything else is shown as a normal file. */
export function parseVoice(name: string | null, size: number | null): { seconds: number; type: string } | null {
  const m = name ? NAME.exec(name) : null;
  if (!m || !size || size > MAX_FILE_BYTES) return null;
  const seconds = Math.min(MAX_VOICE_SECONDS, Number(m[2]));
  return { seconds, type: FORMATS.find((f) => f.ext === m[3])!.type };
}

export type Clip = { blob: Blob; seconds: number; name: string };

export class VoiceError extends Error {}

/** One recording. Call `stop()` to get the clip or `cancel()` to throw it away; both free the microphone. */
export class Recorder {
  private stream: MediaStream | null = null;
  private rec: MediaRecorder | null = null;
  private chunks: Blob[] = [];
  private started = 0;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private fmt = pickFormat();
  /** Called when the time limit stops the recording by itself. */
  onLimit: (() => void) | null = null;

  async start(): Promise<void> {
    if (!this.fmt) throw new VoiceError("Voice notes are not supported on this system.");
    try {
      this.stream = await navigator.mediaDevices.getUserMedia({ audio: { echoCancellation: true, noiseSuppression: true } });
    } catch (e) {
      const n = e instanceof DOMException ? e.name : "";
      if (n === "NotFoundError") throw new VoiceError("No microphone was found.");
      throw new VoiceError("Microphone access was blocked.");
    }
    this.rec = new MediaRecorder(this.stream, { mimeType: this.fmt.mime, audioBitsPerSecond: BITRATE });
    this.rec.ondataavailable = (e) => e.data.size > 0 && this.chunks.push(e.data);
    this.rec.start(250);
    this.started = Date.now();
    this.timer = setTimeout(() => this.onLimit?.(), MAX_VOICE_SECONDS * 1000);
  }

  elapsed(): number {
    return this.started ? (Date.now() - this.started) / 1000 : 0;
  }

  stop(): Promise<Clip> {
    const { rec, fmt } = this;
    if (!rec || !fmt) return Promise.reject(new VoiceError("Not recording."));
    const seconds = Math.max(1, Math.min(MAX_VOICE_SECONDS, Math.round(this.elapsed())));
    return new Promise((resolve, reject) => {
      rec.onstop = () => {
        this.release();
        const blob = new Blob(this.chunks, { type: fmt.type });
        this.chunks = [];
        if (blob.size === 0) return reject(new VoiceError("Nothing was recorded."));
        if (blob.size > MAX_FILE_BYTES) return reject(new VoiceError("That recording is too large."));
        resolve({ blob, seconds, name: `voice-${Math.floor(Date.now() / 1000)}-${seconds}s.${fmt.ext}` });
      };
      rec.onerror = () => {
        this.release();
        reject(new VoiceError("Recording failed."));
      };
      if (rec.state === "inactive") rec.onstop?.(new Event("stop"));
      else rec.stop();
    });
  }

  cancel(): void {
    if (this.rec && this.rec.state !== "inactive") {
      this.rec.onstop = null;
      this.rec.stop();
    }
    this.chunks = [];
    this.release();
  }

  /** Stop the microphone right away: the browser's recording indicator must go out. */
  private release(): void {
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
    this.stream?.getTracks().forEach((t) => t.stop());
    this.stream = null;
    this.rec = null;
  }
}

export function clock(seconds: number): string {
  const s = Math.max(0, Math.round(seconds));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

export function blobToBase64(blob: Blob): Promise<string> {
  return blob.arrayBuffer().then((ab) => {
    const buf = new Uint8Array(ab);
    let bin = "";
    for (let i = 0; i < buf.length; i += 0x8000) bin += String.fromCharCode(...buf.subarray(i, i + 0x8000));
    return btoa(bin);
  });
}

export function base64ToBlob(b64: string, type: string): Blob {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return new Blob([out], { type });
}
