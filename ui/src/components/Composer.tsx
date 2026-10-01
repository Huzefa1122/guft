import { useEffect, useRef, useState, type ChangeEvent, type KeyboardEvent } from "react";
import { Mic, Paperclip, SendHorizontal, Square, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { MAX_FILE_BYTES } from "@/lib/api";
import { bytes, fileToBase64 } from "@/lib/format";
import { useApp } from "@/lib/store";
import { blobToBase64, clock, MAX_VOICE_SECONDS, Recorder, voiceSupported, VoiceError, type Clip } from "@/lib/voice";
import { VoicePlayer } from "./VoicePlayer";

const MAX_TEXT = 8 * 1024;

type Voice = { k: "off" } | { k: "recording" } | { k: "review"; clip: Clip; url: string };

export function Composer() {
  const { send, sendFile, toast, selected } = useApp();
  const [text, setText] = useState("");
  const [voice, setVoice] = useState<Voice>({ k: "off" });
  const [secs, setSecs] = useState(0);
  const [sending, setSending] = useState(false);
  const rec = useRef<Recorder | null>(null);
  const area = useRef<HTMLTextAreaElement>(null);
  const picker = useRef<HTMLInputElement>(null);
  const tooLong = new TextEncoder().encode(text).length > MAX_TEXT;
  const canVoice = voiceSupported();

  // A recording belongs to the chat it was started in: switching chats (or locking) throws it away.
  useEffect(() => discard, [selected]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (voice.k !== "recording") return;
    const t = setInterval(() => setSecs(rec.current?.elapsed() ?? 0), 200);
    return () => clearInterval(t);
  }, [voice.k]);

  function discard() {
    rec.current?.cancel();
    rec.current = null;
    setVoice((v) => {
      if (v.k === "review") URL.revokeObjectURL(v.url);
      return { k: "off" };
    });
    setSecs(0);
  }

  async function startVoice() {
    const r = new Recorder();
    try {
      await r.start();
    } catch (e) {
      toast(e instanceof VoiceError ? e.message : "Couldn't start recording.", "error");
      return;
    }
    rec.current = r;
    r.onLimit = () => void stopVoice();
    setSecs(0);
    setVoice({ k: "recording" });
  }

  async function stopVoice() {
    const r = rec.current;
    if (!r) return;
    rec.current = null;
    try {
      const clip = await r.stop();
      setVoice({ k: "review", clip, url: URL.createObjectURL(clip.blob) });
    } catch (e) {
      setVoice({ k: "off" });
      toast(e instanceof VoiceError ? e.message : "Recording failed.", "error");
    }
  }

  async function sendVoice() {
    if (voice.k !== "review") return;
    const { clip, url } = voice;
    setSending(true);
    try {
      await sendFile(clip.name, await blobToBase64(clip.blob));
      URL.revokeObjectURL(url);
      setVoice({ k: "off" });
    } finally {
      setSending(false);
    }
  }

  function grow() {
    const el = area.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 140)}px`;
  }

  async function submit() {
    const t = text.trim();
    if (!t || tooLong) return;
    setText("");
    requestAnimationFrame(grow);
    await send(t);
  }

  function onKey(e: KeyboardEvent<HTMLTextAreaElement>) {
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      void submit();
    }
  }

  async function onFile(e: ChangeEvent<HTMLInputElement>) {
    const file = e.target.files?.[0];
    e.target.value = "";
    if (!file) return;
    if (file.size > MAX_FILE_BYTES) {
      toast(`"${file.name}" is ${bytes(file.size)}. Files must be smaller than 1 MB.`, "error");
      return;
    }
    await sendFile(file.name, await fileToBase64(file));
  }

  if (voice.k === "recording") {
    return (
      <div className="flex items-center gap-3 border-t bg-card px-4 py-3" role="group" aria-label="Recording a voice note">
        <Button variant="ghost" size="icon" className="size-10 shrink-0 text-muted-foreground" onClick={discard} aria-label="Discard recording">
          <Trash2 className="size-5" />
        </Button>
        <div className="flex min-w-0 flex-1 items-center gap-3 rounded-2xl bg-secondary px-4 py-2.5" role="status">
          <span className="size-2.5 shrink-0 animate-pulse rounded-full bg-destructive" aria-hidden />
          <span className="text-[15px] tabular-nums">{clock(secs)}</span>
          <span className="text-xs text-muted-foreground">of {clock(MAX_VOICE_SECONDS)} · recording</span>
        </div>
        <Button size="icon" className="size-10 shrink-0 rounded-full" onClick={() => void stopVoice()} aria-label="Stop recording">
          <Square className="size-4 fill-current" />
        </Button>
      </div>
    );
  }

  if (voice.k === "review") {
    return (
      <div className="flex items-center gap-3 border-t bg-card px-4 py-3" role="group" aria-label="Voice note ready to send">
        <Button variant="ghost" size="icon" className="size-10 shrink-0 text-muted-foreground" onClick={discard} disabled={sending} aria-label="Discard voice note">
          <Trash2 className="size-5" />
        </Button>
        <div className="min-w-0 flex-1 rounded-2xl bg-secondary px-4 py-2">
          <VoicePlayer load={async () => voice.url} seconds={voice.clip.seconds} onError={(m) => toast(m, "error")} reserve={false} />
        </div>
        <Button size="icon" className="size-10 shrink-0 rounded-full" onClick={() => void sendVoice()} disabled={sending} aria-label="Send voice note">
          <SendHorizontal className="size-5" />
        </Button>
      </div>
    );
  }

  const hasText = text.trim().length > 0;
  return (
    <div className="flex items-end gap-2 border-t bg-card px-4 py-3">
      <input ref={picker} type="file" className="hidden" onChange={onFile} />
      <Button variant="ghost" size="icon" className="size-10 shrink-0 text-muted-foreground" onClick={() => picker.current?.click()} aria-label="Attach a file (up to 1 MB)" title="Attach a file (up to 1 MB)">
        <Paperclip className="size-5" />
      </Button>
      <div className="relative min-w-0 flex-1">
        <textarea
          ref={area}
          value={text}
          rows={1}
          onChange={(e) => { setText(e.target.value); grow(); }}
          onKeyDown={onKey}
          placeholder="Type a message"
          aria-label="Message"
          className="selectable block max-h-36 w-full resize-none rounded-2xl border-0 bg-secondary px-4 py-2.5 text-[15px] leading-snug outline-none placeholder:text-muted-foreground focus-visible:ring-2 focus-visible:ring-ring/60"
        />
        {tooLong && <p className="absolute -top-5 right-2 text-xs text-destructive">Too long (8 kB max)</p>}
      </div>
      {hasText || !canVoice ? (
        <Button size="icon" className="size-10 shrink-0 rounded-full" onClick={() => void submit()} disabled={!hasText || tooLong} aria-label="Send">
          <SendHorizontal className="size-5" />
        </Button>
      ) : (
        <Button size="icon" className="size-10 shrink-0 rounded-full" onClick={() => void startVoice()} aria-label="Record a voice note" title="Record a voice note (up to 1 minute)">
          <Mic className="size-5" />
        </Button>
      )}
    </div>
  );
}
