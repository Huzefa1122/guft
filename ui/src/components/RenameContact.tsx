import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { api, errorText } from "@/lib/api";
import { useApp } from "@/lib/store";

/** A name only you see. The contact is never told. */
export function RenameContactDialog({ id, current, open, onOpenChange }: { id: string; current: string; open: boolean; onOpenChange: (o: boolean) => void }) {
  const { toast, refresh } = useApp();
  const [name, setName] = useState(current);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (open) setName(current);
  }, [open, current]);

  async function save() {
    setBusy(true);
    try {
      await api.renameContact(id, name.trim());
      await refresh();
      onOpenChange(false);
    } catch (e) {
      toast(errorText(e), "error");
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-sm">
        <DialogHeader>
          <DialogTitle>Rename contact</DialogTitle>
          <DialogDescription>Only you see this name. {current} is not told.</DialogDescription>
        </DialogHeader>
        <form onSubmit={(e) => { e.preventDefault(); if (name.trim()) void save(); }} className="space-y-4">
          <Input value={name} maxLength={48} onChange={(e) => setName(e.target.value)} aria-label="Contact name" autoFocus />
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)} disabled={busy}>Cancel</Button>
            <Button type="submit" disabled={busy || !name.trim() || name.trim() === current}>Save</Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
