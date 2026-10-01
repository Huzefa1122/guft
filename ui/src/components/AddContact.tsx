import { useEffect, useState } from "react";
import { Check, Copy, Loader2, Share2, UserPlus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";
import { api, errorText, type Invite } from "@/lib/api";
import { useApp } from "@/lib/store";

const TTLS = [
  { minutes: 15, label: "15 minutes" },
  { minutes: 60, label: "1 hour" },
  { minutes: 24 * 60, label: "24 hours" },
  { minutes: 7 * 24 * 60, label: "7 days" },
];

export function CopyButton({ text, label }: { text: string; label: string }) {
  const [done, setDone] = useState(false);
  const { toast } = useApp();
  async function copy() {
    try {
      await navigator.clipboard.writeText(text);
      setDone(true);
      setTimeout(() => setDone(false), 1500);
    } catch {
      toast("Couldn't copy. Select the text and copy it manually.", "error");
    }
  }
  return (
    <Button variant="outline" size="sm" onClick={() => void copy()} aria-label={label}>
      {done ? <Check className="size-4 text-primary" /> : <Copy className="size-4" />}
      {done ? "Copied" : "Copy"}
    </Button>
  );
}

function Share() {
  const { toast } = useApp();
  const [label, setLabel] = useState("");
  const [ttl, setTtl] = useState(24 * 60);
  const [busy, setBusy] = useState(false);
  const [invite, setInvite] = useState<Invite | null>(null);

  async function make() {
    setBusy(true);
    try {
      setInvite(await api.newInvite(label.trim() || "New contact", ttl));
    } catch (e) {
      toast(errorText(e), "error");
    } finally {
      setBusy(false);
    }
  }

  if (invite) {
    return (
      <div className="space-y-4">
        <div>
          <div className="mb-1.5 flex items-center justify-between">
            <span className="text-sm font-medium">1 · Invite</span>
            <CopyButton text={invite.invite} label="Copy invite" />
          </div>
          <Textarea readOnly value={invite.invite} rows={4} className="selectable font-mono text-xs break-all" onFocus={(e) => e.currentTarget.select()} aria-label="Invite" />
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
          The invite works <strong className="text-foreground">once</strong> and expires in {TTLS.find((t) => t.minutes === ttl)?.label}. When they use it, this person appears in your chats.
        </p>
        <Button variant="outline" className="w-full" onClick={() => setInvite(null)}>Create another invite</Button>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <div>
        <label htmlFor="who" className="mb-1.5 block text-sm font-medium">Who is it for?</label>
        <Input id="who" value={label} maxLength={48} onChange={(e) => setLabel(e.target.value)} placeholder="A label only you see (optional)" />
      </div>
      <div>
        <label htmlFor="ttl" className="mb-1.5 block text-sm font-medium">Expires after</label>
        <Select value={String(ttl)} onValueChange={(v) => setTtl(Number(v))}>
          <SelectTrigger id="ttl"><SelectValue /></SelectTrigger>
          <SelectContent>
            {TTLS.map((t) => (<SelectItem key={t.minutes} value={String(t.minutes)}>{t.label}</SelectItem>))}
          </SelectContent>
        </Select>
      </div>
      <Button className="w-full" onClick={() => void make()} disabled={busy}>
        {busy ? <Loader2 className="size-4 animate-spin" /> : <Share2 className="size-4" />} Create invite
      </Button>
    </div>
  );
}

function Use({ done }: { done: () => void }) {
  const { toast, select, refresh } = useApp();
  const [invite, setInvite] = useState("");
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [isRoom, setIsRoom] = useState(false);
  const [separate, setSeparate] = useState(false);
  const [alias, setAlias] = useState("");
  const ok = invite.trim().length > 20 && code.replace(/\W/g, "").length >= 12 && (!separate || alias.trim().length > 0);

  // Only a room invite can be used with a separate identity.
  useEffect(() => {
    const text = invite.trim();
    if (text.length < 20) {
      setIsRoom(false);
      return;
    }
    let live = true;
    api.inviteIsRoom(text).then((r) => live && setIsRoom(r)).catch(() => live && setIsRoom(false));
    return () => {
      live = false;
    };
  }, [invite]);
  useEffect(() => {
    if (!isRoom) setSeparate(false);
  }, [isRoom]);

  async function add() {
    setBusy(true);
    setError("");
    try {
      const id = await api.addContact(invite.trim(), code.trim(), separate ? alias.trim() : undefined);
      await refresh();
      if (separate) {
        toast("Joining as a separate identity. The room appears once the host's app lets you in.");
      } else {
        select(id);
        toast("Contact added. They'll appear once their app accepts.");
      }
      done();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="space-y-4">
      <div>
        <label htmlFor="inv" className="mb-1.5 block text-sm font-medium">Invite</label>
        <Textarea id="inv" value={invite} onChange={(e) => setInvite(e.target.value)} rows={4} placeholder="Paste the invite (starts with guft1:)" className="selectable font-mono text-xs break-all" spellCheck={false} />
      </div>
      <div>
        <label htmlFor="code" className="mb-1.5 block text-sm font-medium">One-time code</label>
        <Input id="code" value={code} onChange={(e) => setCode(e.target.value)} placeholder="xxxx-xxxx-xxxx" className="font-mono" autoComplete="off" spellCheck={false} />
      </div>
      {isRoom && (
        <div className="space-y-3 rounded-lg border p-3">
          <div className="flex items-start justify-between gap-4">
            <div>
              <label htmlFor="join-separate" className="text-sm font-medium">Join as a separate identity</label>
              <p className="mt-0.5 text-xs text-muted-foreground">
                A new name, new keys and a new Tor address just for this room. Nothing links it to you elsewhere, and it is erased when you leave.
              </p>
            </div>
            <Switch id="join-separate" checked={separate} onCheckedChange={setSeparate} />
          </div>
          {separate && (
            <div>
              <label htmlFor="join-alias" className="mb-1.5 block text-sm font-medium">Your name in this room</label>
              <Input id="join-alias" value={alias} maxLength={48} onChange={(e) => setAlias(e.target.value)} placeholder="e.g. Mask" />
            </div>
          )}
        </div>
      )}
      {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
      <Button className="w-full" onClick={() => void add()} disabled={!ok || busy}>
        {busy ? <Loader2 className="size-4 animate-spin" /> : <UserPlus className="size-4" />} {isRoom ? "Join room" : "Add contact"}
      </Button>
    </div>
  );
}

export function AddContactDialog({ open, onOpenChange }: { open: boolean; onOpenChange: (o: boolean) => void }) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>Add a contact</DialogTitle>
          <DialogDescription>There's no directory. People connect through a one-time invite plus a separate code.</DialogDescription>
        </DialogHeader>
        <Tabs defaultValue="share">
          <TabsList className="w-full">
            <TabsTrigger value="share" className="flex-1">Invite someone</TabsTrigger>
            <TabsTrigger value="use" className="flex-1">I have an invite</TabsTrigger>
          </TabsList>
          <TabsContent value="share" className="pt-3"><Share /></TabsContent>
          <TabsContent value="use" className="pt-3"><Use done={() => onOpenChange(false)} /></TabsContent>
        </Tabs>
      </DialogContent>
    </Dialog>
  );
}
