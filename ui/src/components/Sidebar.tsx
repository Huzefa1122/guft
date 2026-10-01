import { useMemo, useState } from "react";
import { Lock, Search, Settings as SettingsIcon, ShieldCheck, UserPlus, UsersRound } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { bytes, listTime } from "@/lib/format";
import { useApp, type Conv, type Net } from "@/lib/store";
import { clock, parseVoice } from "@/lib/voice";
import { cn } from "@/lib/utils";
import { Brand } from "./Brand";
import { ContactAvatar } from "./ContactAvatar";
import { RoomAvatar } from "./RoomAvatar";
import { Ticks } from "./Ticks";

const NET_LABEL: Record<Net, string> = {
  off: "Offline",
  starting: "Connecting to Tor…",
  online: "Connected over Tor",
  error: "Network problem",
};

function NetDot() {
  const { net, netMessage } = useApp();
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span className="flex items-center gap-1.5 rounded-full px-2 py-1 text-xs text-muted-foreground" role="status">
          <span
            className={cn(
              "size-2 rounded-full",
              net === "online" && "bg-primary",
              net === "starting" && "animate-pulse bg-amber-500",
              net === "error" && "bg-destructive",
              net === "off" && "bg-muted-foreground/50",
            )}
          />
          {net === "online" ? "Online" : net === "starting" ? "Connecting" : net === "error" ? "Offline" : "Locked"}
        </span>
      </TooltipTrigger>
      <TooltipContent>{netMessage ? `${NET_LABEL[net]}: ${netMessage}` : NET_LABEL[net]}</TooltipContent>
    </Tooltip>
  );
}

function preview(conv: Conv): string {
  const last = conv.last;
  if (!last) return conv.temp ? "Nothing saved. Say something." : "No messages yet";
  const voice = last.kind === "file" ? parseVoice(last.fileName, last.fileSize) : null;
  const body = voice ? `🎤 Voice note (${clock(voice.seconds)})` : last.kind === "file" ? `📎 ${last.fileName} (${bytes(last.fileSize ?? 0)})` : last.text;
  if (conv.kind === "room" && !conv.direct && !last.outgoing && last.sender) {
    const who = conv.members.find((m) => m.id === last.sender)?.name;
    return who ? `${who}: ${body}` : body ?? "";
  }
  return body ?? "";
}

export function Sidebar({ onAdd, onAddRoom, onSettings }: { onAdd: () => void; onAddRoom: () => void; onSettings: () => void }) {
  const { convs, selected, select, lock, status } = useApp();
  const [q, setQ] = useState("");
  const shown = useMemo(() => {
    const needle = q.trim().toLowerCase();
    return needle ? convs.filter((c) => c.name.toLowerCase().includes(needle)) : convs;
  }, [convs, q]);

  return (
    <aside className="flex h-full w-[360px] shrink-0 flex-col border-r bg-sidebar">
      <header className="flex h-16 shrink-0 items-center justify-between px-4">
        <Brand />
        <div className="flex items-center gap-0.5">
          <NetDot />
          <Tooltip>
            <TooltipTrigger asChild>
              <Button variant="ghost" size="icon" onClick={onAdd} aria-label="Add contact">
                <UserPlus className="size-5" />
              </Button>
            </TooltipTrigger>
            <TooltipContent>Add contact</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger asChild>
              <Button variant="ghost" size="icon" onClick={onAddRoom} aria-label="New room">
                <UsersRound className="size-5" />
              </Button>
            </TooltipTrigger>
            <TooltipContent>New room</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger asChild>
              <Button variant="ghost" size="icon" onClick={() => void lock()} aria-label="Lock now">
                <Lock className="size-5" />
              </Button>
            </TooltipTrigger>
            <TooltipContent>Lock now</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger asChild>
              <Button variant="ghost" size="icon" onClick={onSettings} aria-label="Settings">
                <SettingsIcon className="size-5" />
              </Button>
            </TooltipTrigger>
            <TooltipContent>Settings</TooltipContent>
          </Tooltip>
        </div>
      </header>

      <div className="px-3 pb-2">
        <div className="relative">
          <Search className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input value={q} onChange={(e) => setQ(e.target.value)} placeholder="Search chats" className="h-9 rounded-lg border-0 bg-secondary pl-9 shadow-none" aria-label="Search chats" />
        </div>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto" role="list" aria-label="Chats">
        {convs.length === 0 ? (
          <div className="mx-6 mt-16 flex flex-col items-center gap-3 text-center">
            <div className="grid size-14 place-items-center rounded-full bg-secondary">
              <UserPlus className="size-6 text-muted-foreground" />
            </div>
            <p className="font-medium">No conversations yet</p>
            <p className="text-sm text-muted-foreground">
              Contacts are added with a one-time invite, so nobody can find you without it. Rooms connect a few people at once.
            </p>
            <div className="flex gap-2">
              <Button onClick={onAdd}>Add a contact</Button>
              <Button variant="outline" onClick={onAddRoom}>New room</Button>
            </div>
          </div>
        ) : shown.length === 0 ? (
          <p className="mt-10 text-center text-sm text-muted-foreground">No matches</p>
        ) : (
          shown.map((conv) => (
            <button
              key={conv.id}
              role="listitem"
              onClick={() => select(conv.id)}
              className={cn(
                "flex w-full items-center gap-3 px-3 py-2.5 text-left transition-colors hover:bg-accent",
                selected === conv.id && "bg-accent",
              )}
            >
              {conv.kind === "room" ? <RoomAvatar name={conv.name} temp={conv.temp} /> : <ContactAvatar id={conv.id} name={conv.name} />}
              <div className="min-w-0 flex-1 border-b border-border/60 pb-2.5">
                <div className="flex items-baseline justify-between gap-2">
                  <span className="flex min-w-0 items-center gap-1 truncate font-medium">
                    <span className="truncate">{conv.name}</span>
                    {conv.temp && <span className="shrink-0 rounded bg-secondary px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wide text-muted-foreground">Temp</span>}
                    {conv.kind === "contact" && conv.verified && <ShieldCheck className="size-3.5 shrink-0 text-primary" aria-label="Verified" />}
                  </span>
                  {conv.last && <span className={cn("shrink-0 text-xs", conv.unread ? "font-medium text-primary" : "text-muted-foreground")}>{listTime(conv.last.ts)}</span>}
                </div>
                <div className="mt-0.5 flex items-center justify-between gap-2">
                  <span className="flex min-w-0 items-center gap-1 text-sm text-muted-foreground">
                    {conv.direct ? (
                      <span className="truncate">{preview(conv)}</span>
                    ) : conv.kind === "room" ? (
                      <span className="truncate">{conv.members.length + 1} member{conv.members.length + 1 === 1 ? "" : "s"} · {preview(conv)}</span>
                    ) : (
                      <>
                        {conv.last?.outgoing && <Ticks status={conv.last.status} />}
                        <span className="truncate">{preview(conv)}</span>
                      </>
                    )}
                  </span>
                  {conv.unread > 0 && (
                    <span className="grid h-5 min-w-5 shrink-0 place-items-center rounded-full bg-primary px-1.5 text-xs font-medium text-primary-foreground" aria-label={`${conv.unread} unread`}>
                      {conv.unread}
                    </span>
                  )}
                </div>
              </div>
            </button>
          ))
        )}
      </div>
      <footer className="border-t px-4 py-2.5 text-xs text-muted-foreground">
        Signed in as <span className="font-medium text-foreground">{status?.name}</span>
      </footer>
    </aside>
  );
}
