import { Check, Copy, Monitor, Moon, Sun } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Separator } from "@/components/ui/separator";
import { Switch } from "@/components/ui/switch";
import { api, errorText } from "@/lib/api";
import { useApp, type Theme } from "@/lib/store";
import { cn } from "@/lib/utils";
import { useState } from "react";

const IDLE = [1, 5, 15, 30, 60];
const THEMES: { id: Theme; label: string; icon: typeof Sun }[] = [
  { id: "system", label: "System", icon: Monitor },
  { id: "light", label: "Light", icon: Sun },
  { id: "dark", label: "Dark", icon: Moon },
];

export function SettingsDialog({ open, onOpenChange }: { open: boolean; onOpenChange: (o: boolean) => void }) {
  const { status, refresh, toast, theme, setTheme, idleMinutes, setIdleMinutes } = useApp();
  const [copied, setCopied] = useState(false);

  async function setOnline(on: boolean) {
    try {
      await api.setOnlineWhenLocked(on);
      await refresh();
    } catch (e) {
      toast(errorText(e), "error");
    }
  }

  async function copyOnion() {
    if (!status?.onion) return;
    try {
      await navigator.clipboard.writeText(status.onion);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      toast("Couldn't copy to the clipboard", "error");
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex max-h-[min(85vh,34rem)] max-w-md flex-col gap-0 overflow-hidden p-0">
        <DialogHeader className="shrink-0 px-6 pt-6 pb-3">
          <DialogTitle>Settings</DialogTitle>
          <DialogDescription>Stored on this device only.</DialogDescription>
        </DialogHeader>

        {/* Only the body scrolls, and only vertically: the header stays put and nothing spills sideways. */}
        <div className="min-h-0 flex-1 space-y-4 overflow-x-hidden overflow-y-auto px-6 pb-6">
        <section className="space-y-3">
          <h3 className="text-sm font-medium">Privacy &amp; locking</h3>
          <div className="flex items-center justify-between gap-4">
            <label htmlFor="idle" className="text-sm">Lock after being idle for</label>
            <Select value={String(idleMinutes)} onValueChange={(v) => setIdleMinutes(Number(v))}>
              <SelectTrigger id="idle" className="w-40"><SelectValue /></SelectTrigger>
              <SelectContent>
                {IDLE.map((m) => (<SelectItem key={m} value={String(m)}>{m} {m === 1 ? "minute" : "minutes"}</SelectItem>))}
              </SelectContent>
            </Select>
          </div>
          <div className="flex items-start justify-between gap-4">
            <div>
              <label htmlFor="online" className="text-sm">Stay reachable while locked</label>
              <p className="mt-0.5 text-xs text-muted-foreground">
                Incoming messages are received and sealed with a post-quantum key, and are only readable after you unlock. Your Tor address key stays in memory while locked; your passphrase key and message keys do not.
              </p>
            </div>
            <Switch id="online" checked={status?.onlineWhenLocked ?? false} onCheckedChange={(v) => void setOnline(v)} />
          </div>
        </section>

        <Separator />
        <section className="space-y-3">
          <h3 className="text-sm font-medium">Appearance</h3>
          <div className="grid grid-cols-3 gap-2" role="radiogroup" aria-label="Theme">
            {THEMES.map(({ id, label, icon: Icon }) => (
              <button key={id} role="radio" aria-checked={theme === id} onClick={() => setTheme(id)} className={cn("flex items-center justify-center gap-2 rounded-lg border px-3 py-2 text-sm transition-colors hover:bg-accent", theme === id && "border-primary bg-accent")}>
                <Icon className="size-4" /> {label}
              </button>
            ))}
          </div>
        </section>

        <Separator />
        <section className="space-y-2">
          <h3 className="text-sm font-medium">Your Tor address</h3>
          <div className="flex items-center gap-2">
            <code className="selectable min-w-0 flex-1 truncate rounded-md bg-secondary px-2.5 py-2 text-xs">{status?.onion ?? "…"}</code>
            <Button variant="outline" size="sm" onClick={() => void copyOnion()} aria-label="Copy address">
              {copied ? <Check className="size-4 text-primary" /> : <Copy className="size-4" />}
            </Button>
          </div>
          <p className="text-xs text-muted-foreground">Contacts reach you here. It's hidden from anyone who hasn't been invited.</p>
        </section>

        <Separator />
        <section className="space-y-1 text-xs text-muted-foreground">
          <h3 className="text-sm font-medium text-foreground">How your messages are protected</h3>
          <p>Signal-protocol encryption with a post-quantum handshake (ML-KEM-1024 + X25519) and a post-quantum ratchet, wrapped in a second layer keyed from your invite code, sent over Tor onion services in fixed-size cells across several circuits.</p>
          <p>Open source (AGPL-3.0). Not independently audited yet.</p>
        </section>
        </div>
      </DialogContent>
    </Dialog>
  );
}
