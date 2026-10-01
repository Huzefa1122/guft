import { ChevronDown, Copy, Download, FileText, Trash2 } from "lucide-react";
import type { CSSProperties } from "react";
import { Button } from "@/components/ui/button";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { api, errorText, type Message } from "@/lib/api";
import { bytes, clock, hueOf } from "@/lib/format";
import { base64ToBlob, parseVoice } from "@/lib/voice";
import { useApp } from "@/lib/store";
import { cn } from "@/lib/utils";
import { Ticks } from "./Ticks";
import { VoicePlayer } from "./VoicePlayer";

export function Bubble({ m, first, senderName, showSender = false }: { m: Message; first: boolean; senderName?: string; showSender?: boolean }) {
  const { toast, select, selected, refresh } = useApp();

  async function save() {
    try {
      const path = await api.saveFile(m.id);
      toast(`Saved to ${path}`);
    } catch (e) {
      toast(errorText(e), "error");
    }
  }

  async function copyText() {
    try {
      await navigator.clipboard.writeText(m.text ?? "");
      toast("Copied");
    } catch {
      toast("Couldn't copy. Select the text and copy it manually.", "error");
    }
  }

  async function remove() {
    try {
      await api.deleteMessage(m.id);
      if (selected) select(selected);
      await refresh();
    } catch (e) {
      toast(errorText(e), "error");
    }
  }

  const voice = m.kind === "file" ? parseVoice(m.fileName, m.fileSize) : null;
  const loadVoice = async () => URL.createObjectURL(base64ToBlob(await api.fileBytes(m.id), voice!.type));

  return (
    <div className={cn("flex", m.outgoing ? "justify-end" : "justify-start", first ? "mt-2" : "mt-0.5")}>
      <div
        className={cn(
          "group/bubble selectable relative max-w-[min(34rem,75%)] rounded-xl px-3 py-1.5 shadow-[0_1px_0.5px_rgb(0_0_0/0.12)]",
          m.outgoing ? "bg-bubble-out text-primary-foreground" : "bg-bubble-in",
          first && (m.outgoing ? "rounded-tr-none" : "rounded-tl-none"),
        )}
      >
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <button
              className={cn(
                "absolute top-0.5 right-1 z-10 hidden size-6 place-items-center rounded-full opacity-70 hover:opacity-100 focus-visible:grid group-hover/bubble:grid data-[state=open]:grid",
                m.outgoing ? "bg-bubble-out" : "bg-bubble-in",
              )}
              aria-label="Message options"
            >
              <ChevronDown className="size-4" />
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align={m.outgoing ? "end" : "start"}>
            {m.kind === "text" && (
              <DropdownMenuItem onSelect={() => void copyText()}>
                <Copy /> Copy text
              </DropdownMenuItem>
            )}
            <DropdownMenuItem variant="destructive" onSelect={() => void remove()}>
              <Trash2 /> {m.status === "queued" ? "Cancel and delete" : "Delete for me"}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
        {showSender && senderName && (
          <p className="sender-name mb-0.5 truncate text-xs font-semibold" style={{ "--sender-hue": hueOf(m.sender ?? senderName) } as CSSProperties}>
            {senderName}
          </p>
        )}
        {voice ? (
          <div className="relative">
            <VoicePlayer load={loadVoice} seconds={voice.seconds} tone={m.outgoing ? "out" : "in"} onError={(t) => toast(t, "error")} />
            <Button variant="ghost" size="icon" className="absolute -top-0.5 right-0 size-7 opacity-60 hover:opacity-100" onClick={save} aria-label="Save voice note">
              <Download className="size-3.5" />
            </Button>
          </div>
        ) : m.kind === "file" ? (
          <div className="flex min-w-56 items-center gap-3 pr-14 pb-1">
            <div className={cn("grid size-11 shrink-0 place-items-center rounded-lg", m.outgoing ? "bg-white/15 dark:bg-black/10" : "bg-black/5 dark:bg-white/10")}>
              <FileText className="size-5 opacity-70" />
            </div>
            <div className="min-w-0 flex-1">
              <p className="truncate text-sm font-medium" title={m.fileName ?? ""}>{m.fileName}</p>
              <p className={cn("text-xs", m.outgoing ? "opacity-70" : "text-muted-foreground")}>{bytes(m.fileSize ?? 0)}</p>
            </div>
            <Button variant="ghost" size="icon" className="size-8 opacity-80" onClick={save} aria-label={`Save ${m.fileName}`}>
              <Download className="size-4" />
            </Button>
          </div>
        ) : (
          <p className="text-[15px] leading-snug break-words whitespace-pre-wrap">
            {m.text}
            {/* Reserves room for the time so it never overlaps the last line. */}
            <span className="inline-block w-16" aria-hidden />
          </p>
        )}
        <span className={cn("absolute right-2 bottom-1 flex items-center gap-1 text-[11px]", m.outgoing ? "opacity-70" : "text-muted-foreground")}>
          {clock(m.ts)}
          {m.outgoing && <Ticks status={m.status} className={m.outgoing ? "text-current" : undefined} />}
        </span>
      </div>
    </div>
  );
}
