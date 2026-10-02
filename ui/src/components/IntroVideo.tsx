import { useEffect, useState } from "react";
import { Loader2, Play } from "lucide-react";

/**
 * The 35-second intro. It ships inside the app (nothing is fetched from the internet): the poster is
 * shown first and the video is only loaded when the user presses play, through a blob URL (the CSP
 * allows `media-src blob:`).
 */
export function IntroVideo({ className = "" }: { className?: string }) {
  const [src, setSrc] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);

  useEffect(() => () => { if (src) URL.revokeObjectURL(src); }, [src]);

  async function load() {
    if (busy) return;
    setBusy(true);
    setFailed(false);
    try {
      const res = await fetch("/intro.mp4");
      if (!res.ok) throw new Error("missing");
      setSrc(URL.createObjectURL(await res.blob()));
    } catch {
      setFailed(true);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className={`relative aspect-video w-full overflow-hidden rounded-xl border bg-black ${className}`}>
      {src ? (
        <video src={src} controls autoPlay playsInline className="h-full w-full" aria-label="guft intro video" />
      ) : (
        <button type="button" onClick={() => void load()} className="group absolute inset-0 grid place-items-center" aria-label="Play the 35 second intro">
          <img src="/intro.jpg" alt="" className="absolute inset-0 h-full w-full object-cover opacity-80 transition-opacity group-hover:opacity-100" draggable={false} />
          <span className="relative grid size-14 place-items-center rounded-full bg-white/90 text-black shadow-lg transition-transform group-hover:scale-105">
            {busy ? <Loader2 className="size-6 animate-spin" /> : <Play className="size-6 translate-x-0.5" />}
          </span>
          {failed && <span className="absolute bottom-2 text-xs text-white/80">Couldn't load the video</span>}
        </button>
      )}
    </div>
  );
}
