import { useEffect, useRef, useState } from "react";
import { Loader2, Pause, Play } from "lucide-react";
import { cn } from "@/lib/utils";
import { clock } from "@/lib/voice";

/**
 * Plays one voice note. The audio is only fetched and decoded when the user taps play,
 * never automatically: a received file is not handed to the media stack unasked.
 */
export function VoicePlayer({
  load,
  seconds,
  onError,
  tone = "in",
  reserve = true,
}: {
  /** Returns a playable URL (a blob URL). Called on first play. */
  load: () => Promise<string>;
  seconds: number;
  onError: (message: string) => void;
  tone?: "in" | "out";
  /** Leave room on the right for the message time (inside a bubble). */
  reserve?: boolean;
}) {
  const audio = useRef<HTMLAudioElement | null>(null);
  const url = useRef<string | null>(null);
  const [state, setState] = useState<"idle" | "loading" | "playing" | "paused">("idle");
  const [at, setAt] = useState(0);

  useEffect(
    () => () => {
      audio.current?.pause();
      audio.current = null;
      if (url.current) URL.revokeObjectURL(url.current);
    },
    [],
  );

  async function toggle() {
    if (state === "loading") return;
    if (audio.current) {
      if (state === "playing") {
        audio.current.pause();
        setState("paused");
      } else {
        await audio.current.play().catch(() => onError("Couldn't play this voice note."));
      }
      return;
    }
    setState("loading");
    try {
      url.current = await load();
      const a = new Audio(url.current);
      a.preload = "auto";
      a.ontimeupdate = () => setAt(a.currentTime);
      a.onplay = () => setState("playing");
      a.onpause = () => setState((s) => (s === "loading" ? s : "paused"));
      a.onended = () => {
        setState("idle");
        setAt(0);
        a.currentTime = 0;
      };
      a.onerror = () => {
        setState("idle");
        audio.current = null;
        onError("Couldn't play this voice note.");
      };
      audio.current = a;
      // A player that never starts must not leave the button spinning forever.
      await Promise.race([a.play(), new Promise<never>((_, rej) => setTimeout(() => rej(new Error("timeout")), 8000))]);
    } catch {
      audio.current?.pause();
      setState("idle");
      audio.current = null;
      onError("Couldn't play this voice note. You can save it and open it in another player.");
    }
  }

  const progress = Math.min(100, (at / Math.max(seconds, 1)) * 100);
  return (
    <div className={cn("flex items-center gap-3", reserve && "min-w-56 pr-14 pb-1")}>
      <button
        onClick={() => void toggle()}
        className={cn(
          "grid size-10 shrink-0 place-items-center rounded-full transition-colors",
          tone === "out" ? "bg-white/20 hover:bg-white/30 dark:bg-black/15" : "bg-primary text-primary-foreground hover:bg-primary/90",
        )}
        aria-label={state === "playing" ? "Pause voice note" : "Play voice note"}
      >
        {state === "loading" ? <Loader2 className="size-4 animate-spin" /> : state === "playing" ? <Pause className="size-4" /> : <Play className="size-4" />}
      </button>
      <div className="min-w-0 flex-1">
        <div className={cn("h-1.5 overflow-hidden rounded-full", tone === "out" ? "bg-white/25 dark:bg-black/15" : "bg-black/10 dark:bg-white/15")} role="progressbar" aria-valuenow={Math.round(progress)} aria-valuemin={0} aria-valuemax={100}>
          <div className={cn("h-full rounded-full", tone === "out" ? "bg-current" : "bg-primary")} style={{ width: `${progress}%` }} />
        </div>
        <p className={cn("mt-1 text-xs tabular-nums", tone === "out" ? "opacity-70" : "text-muted-foreground")}>{state === "idle" ? clock(seconds) : `${clock(at)} / ${clock(seconds)}`}</p>
      </div>
    </div>
  );
}
