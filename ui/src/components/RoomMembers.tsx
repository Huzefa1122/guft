import { useEffect, useState } from "react";
import { Check, Crown, LogOut, Pencil, Trash2, UserMinus, UserPlus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Separator } from "@/components/ui/separator";
import { api, errorText } from "@/lib/api";
import { useApp, type Conv } from "@/lib/store";
import { cn } from "@/lib/utils";
import { ContactAvatar } from "./ContactAvatar";

export function RoomMembersDialog({
  room,
  open,
  onOpenChange,
  onInvite,
}: {
  room: Conv;
  open: boolean;
  onOpenChange: (o: boolean) => void;
  onInvite: () => void;
}) {
  const { convs, refresh, select, toast } = useApp();
  const [name, setName] = useState(room.name);
  const [picked, setPicked] = useState<string[]>([]);
  const [confirmLeave, setConfirmLeave] = useState(false);
  const [confirmRemove, setConfirmRemove] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (open) {
      setName(room.name);
      setPicked([]);
      setConfirmLeave(false);
      setConfirmRemove(null);
    }
  }, [open, room.name]);

  const contacts = convs.filter((c) => c.kind === "contact");
  const addable = contacts.filter((c) => !room.members.some((m) => m.id === c.id));

  async function run(action: () => Promise<void>, done: string, then?: () => void) {
    setBusy(true);
    try {
      await action();
      await refresh();
      toast(done);
      then?.();
    } catch (e) {
      toast(errorText(e), "error");
    } finally {
      setBusy(false);
    }
  }

  const rename = () => run(() => api.renameRoom(room.id, name.trim()), "Room renamed");
  const leave = () => run(() => api.leaveRoom(room.id), "You left the room", () => { select(null); onOpenChange(false); });
  const remove = (id: string) => run(() => api.removeMember(room.id, id), "Member removed", () => setConfirmRemove(null));
  const add = () =>
    run(
      async () => {
        for (const id of picked) await api.addToRoom(room.id, id);
      },
      picked.length === 1 ? "Member added" : "Members added",
      () => setPicked([]),
    );

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[90vh] max-w-md overflow-y-auto">
        <DialogHeader>
          <DialogTitle>Members of {room.name}</DialogTitle>
          <DialogDescription>
            {room.members.length + 1} member{room.members.length + 1 === 1 ? "" : "s"}, including you. Messages go to everyone over their own encrypted connection.
          </DialogDescription>
        </DialogHeader>

        {room.mine && (
          <div className="flex items-center gap-2">
            <Input value={name} maxLength={48} onChange={(e) => setName(e.target.value)} aria-label="Room name" />
            <Button variant="outline" size="icon" onClick={() => void rename()} disabled={busy || !name.trim() || name.trim() === room.name} aria-label="Rename room">
              <Pencil className="size-4" />
            </Button>
          </div>
        )}

        <div className="space-y-1">
          <div className="flex items-center gap-3 rounded-lg px-2 py-2">
            <ContactAvatar id="me" name="You" className="size-9" />
            <span className="text-sm font-medium">You</span>
            {room.mine && <span className="rounded-full bg-secondary px-2 py-0.5 text-xs text-muted-foreground">Creator</span>}
          </div>
          {room.members.map((m) => (
            <div key={m.id} className="flex items-center gap-3 rounded-lg px-2 py-2">
              <ContactAvatar id={m.id} name={m.name} className="size-9" />
              <div className="min-w-0 flex-1">
                <p className="flex items-center gap-1.5 truncate text-sm font-medium">
                  <span className="truncate">{m.name}</span>
                  {m.isCreator && <Crown className="size-3.5 shrink-0 text-amber-500" aria-label="Creator" />}
                </p>
                <p className="text-xs text-muted-foreground">{m.online ? "Connected" : "Offline"}</p>
              </div>
              {confirmRemove === m.id ? (
                <div className="flex items-center gap-1">
                  <Button variant="ghost" size="sm" onClick={() => setConfirmRemove(null)} disabled={busy}>Cancel</Button>
                  <Button variant="destructive" size="sm" onClick={() => void remove(m.id)} disabled={busy}>Remove</Button>
                </div>
              ) : (
                room.mine &&
                !m.isCreator && (
                  <Button variant="ghost" size="icon" className="size-8 text-muted-foreground" onClick={() => setConfirmRemove(m.id)} aria-label={`Remove ${m.name}`}>
                    <UserMinus className="size-4" />
                  </Button>
                )
              )}
            </div>
          ))}
        </div>

        {room.openInvites || room.mine ? (
          <Button variant="outline" className="w-full" onClick={onInvite} disabled={busy}>
            <UserPlus className="size-4" /> Invite someone new
          </Button>
        ) : null}

        {/* A room with its own identity cannot take people from your main contacts: that would tie them together. */}
        {!room.identity && addable.length > 0 && (room.mine || room.openInvites) && (
          <>
            <Separator />
            <section className="space-y-2">
              <h3 className="text-sm font-medium">Add from your contacts</h3>
              <div className="max-h-40 space-y-0.5 overflow-y-auto">
                {addable.map((c) => {
                  const on = picked.includes(c.id);
                  return (
                    <button
                      key={c.id}
                      type="button"
                      role="checkbox"
                      aria-checked={on}
                      onClick={() => setPicked((p) => (on ? p.filter((x) => x !== c.id) : [...p, c.id]))}
                      className="flex w-full items-center gap-3 rounded-lg px-2 py-1.5 text-left hover:bg-accent"
                    >
                      <ContactAvatar id={c.id} name={c.name} className="size-8" />
                      <span className="min-w-0 flex-1 truncate text-sm">{c.name}</span>
                      <span className={cn("grid size-5 place-items-center rounded-md border", on ? "border-primary bg-primary text-primary-foreground" : "border-input")}>
                        {on && <Check className="size-3.5" />}
                      </span>
                    </button>
                  );
                })}
              </div>
              <Button className="w-full" onClick={() => void add()} disabled={busy || picked.length === 0}>
                <UserPlus className="size-4" /> Add {picked.length > 0 ? picked.length : ""} to room
              </Button>
            </section>
          </>
        )}

        <Separator />
        {confirmLeave ? (
          <div className="flex items-center gap-2">
            <p className="flex-1 text-sm text-muted-foreground">Leave {room.name}?</p>
            <Button variant="ghost" onClick={() => setConfirmLeave(false)} disabled={busy}>Cancel</Button>
            <Button variant="destructive" onClick={() => void leave()} disabled={busy}>
              <LogOut className="size-4" /> Leave
            </Button>
          </div>
        ) : (
          <Button variant="outline" className="w-full text-destructive hover:text-destructive" onClick={() => setConfirmLeave(true)} disabled={busy}>
            <Trash2 className="size-4" /> Leave room
          </Button>
        )}
      </DialogContent>
    </Dialog>
  );
}
