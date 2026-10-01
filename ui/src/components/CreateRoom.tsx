import { useEffect, useState } from "react";
import { Loader2, MessageSquareDashed, UsersRound } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { api, errorText } from "@/lib/api";
import { useApp } from "@/lib/store";

export function CreateRoomDialog({ open, onOpenChange }: { open: boolean; onOpenChange: (o: boolean) => void }) {
  const { toast, refresh, select } = useApp();
  const [name, setName] = useState("");
  const [openInvites, setOpenInvites] = useState(true);
  const [temp, setTemp] = useState(false);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (open) {
      setName("");
      setOpenInvites(true);
      setTemp(false);
    }
  }, [open]);

  async function create() {
    setBusy(true);
    try {
      const id = await api.createRoom(name.trim(), openInvites, temp);
      await refresh();
      select(id);
      toast(temp ? "Temporary room created. It disappears when guft locks or closes." : "Room created. Invite the people you want in it.");
      onOpenChange(false);
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
          <DialogTitle className="flex items-center gap-2">
            <UsersRound className="size-5" /> New room
          </DialogTitle>
          <DialogDescription>
            A private group chat. Everyone in it gets every message directly, encrypted with their own post-quantum session.
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-4">
          <div>
            <label htmlFor="room-name" className="mb-1.5 block text-sm font-medium">Room name</label>
            <Input id="room-name" value={name} maxLength={48} onChange={(e) => setName(e.target.value)} placeholder="e.g. Trip planning" autoFocus />
          </div>
          <div className="flex items-start justify-between gap-4">
            <div>
              <label htmlFor="room-open" className="text-sm">Members can invite</label>
              <p className="mt-0.5 text-xs text-muted-foreground">Off means only you can invite people to this room.</p>
            </div>
            <Switch id="room-open" checked={openInvites} onCheckedChange={setOpenInvites} />
          </div>
          <div className="flex items-start justify-between gap-4">
            <div>
              <label htmlFor="room-temp" className="flex items-center gap-1.5 text-sm">
                <MessageSquareDashed className="size-4" /> Temporary room
              </label>
              <p className="mt-0.5 text-xs text-muted-foreground">
                Messages stay in memory only, on every device. Nothing is saved, and the room is gone when guft locks or closes.
              </p>
            </div>
            <Switch id="room-temp" checked={temp} onCheckedChange={setTemp} />
          </div>
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)} disabled={busy}>Cancel</Button>
          <Button onClick={() => void create()} disabled={busy || !name.trim()}>
            {busy && <Loader2 className="size-4 animate-spin" />} Create room
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
