import { useEffect, useState } from "react";
import { Loader2, Share2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { api, errorText, type Invite } from "@/lib/api";
import { useApp, type Conv } from "@/lib/store";
import { CopyButton } from "./AddContact";

const TTLS = [
  { minutes: 15, label: "15 minutes" },
  { minutes: 60, label: "1 hour" },
  { minutes: 24 * 60, label: "24 hours" },
  { minutes: 7 * 24 * 60, label: "7 days" },
];

/** An invite that also adds whoever uses it to the room. */
export function RoomInviteDialog({ room, open, onOpenChange }: { room: Conv; open: boolean; onOpenChange: (o: boolean) => void }) {
  const { toast, refresh } = useApp();
  const [label, setLabel] = useState("");
  const [ttl, setTtl] = useState(24 * 60);
  const [busy, setBusy] = useState(false);
  const [invite, setInvite] = useState<Invite | null>(null);

  useEffect(() => {
    if (open) {
      setLabel("");
      setTtl(24 * 60);
      setInvite(null);
    }
  }, [open]);

  async function make() {
    setBusy(true);
    try {
      setInvite(await api.roomInvite(room.id, label.trim() || room.name, ttl));
      void refresh();
    } catch (e) {
      toast(errorText(e), "error");
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Invite to {room.name}</DialogTitle>
          <DialogDescription>
            They join this room directly. The invite works once and needs the separate code.
          </DialogDescription>
        </DialogHeader>

        {invite ? (
          <div className="space-y-4">
            <div>
              <div className="mb-1.5 flex items-center justify-between">
                <span className="text-sm font-medium">1 · Invite</span>
                <CopyButton text={invite.invite} label="Copy invite" />
              </div>
              <Textarea readOnly value={invite.invite} rows={4} className="selectable font-mono text-xs break-all" onFocus={(e) => e.currentTarget.select()} aria-label="Room invite" />
              <p className="mt-1 text-xs text-muted-foreground">Send this over any channel (even an insecure one).</p>
            </div>
            <div>
              <div className="mb-1.5 flex items-center justify-between">
                <span className="text-sm font-medium">2 · One-time code</span>
                <CopyButton text={invite.code} label="Copy code" />
              </div>
              <p className="selectable rounded-lg bg-secondary py-3 text-center font-mono text-2xl tracking-widest" aria-label="One-time code">{invite.code}</p>
              <p className="mt-1 text-xs text-muted-foreground">Give this separately, ideally by voice or in person. The invite is useless without it.</p>
            </div>
            <p className="rounded-lg bg-secondary p-3 text-xs text-muted-foreground">
              When they use it, everyone in the room connects to them automatically.
            </p>
            <Button variant="outline" className="w-full" onClick={() => setInvite(null)}>Create another invite</Button>
          </div>
        ) : (
          <div className="space-y-4">
            <div>
              <label htmlFor="rlabel" className="mb-1.5 block text-sm font-medium">Who is it for?</label>
              <Input id="rlabel" value={label} maxLength={48} onChange={(e) => setLabel(e.target.value)} placeholder={`A label only you see (optional)`} />
            </div>
            <div>
              <label htmlFor="rttl" className="mb-1.5 block text-sm font-medium">Expires after</label>
              <Select value={String(ttl)} onValueChange={(v) => setTtl(Number(v))}>
                <SelectTrigger id="rttl"><SelectValue /></SelectTrigger>
                <SelectContent>
                  {TTLS.map((t) => (<SelectItem key={t.minutes} value={String(t.minutes)}>{t.label}</SelectItem>))}
                </SelectContent>
              </Select>
            </div>
            <Button className="w-full" onClick={() => void make()} disabled={busy}>
              {busy ? <Loader2 className="size-4 animate-spin" /> : <Share2 className="size-4" />} Create invite
            </Button>
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}
