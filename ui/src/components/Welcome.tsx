import { useState, type FormEvent } from "react";
import { Loader2, ShieldCheck } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { errorText } from "@/lib/api";
import { strength } from "@/lib/format";
import { useApp } from "@/lib/store";
import { cn } from "@/lib/utils";
import { Brand } from "./Brand";
import { IntroVideo } from "./IntroVideo";

const LABELS = ["Too short", "Weak", "Okay", "Good", "Strong"];
const COLORS = ["bg-destructive", "bg-destructive", "bg-amber-500", "bg-primary", "bg-primary"];

export function Welcome() {
  const { createProfile } = useApp();
  const [name, setName] = useState("");
  const [pass, setPass] = useState("");
  const [again, setAgain] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const score = strength(pass);
  const ok = name.trim().length > 0 && pass.length >= 8 && pass === again;

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!ok || busy) return;
    setBusy(true);
    setError("");
    try {
      await createProfile(pass, name.trim());
    } catch (err) {
      setError(errorText(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="h-full overflow-auto bg-background p-6">
     <div className="mx-auto flex min-h-full w-full max-w-md flex-col justify-center gap-4">
      <form onSubmit={submit} className="w-full rounded-2xl border bg-card p-8 shadow-sm">
        <div className="mb-6 flex flex-col items-center gap-2 text-center">
          <Brand size="lg" />
          <p className="text-sm text-muted-foreground">Private chat with no servers, no accounts and no phone numbers.</p>
        </div>

        <label htmlFor="name" className="mb-1.5 block text-sm font-medium">Your name</label>
        <Input id="name" value={name} maxLength={48} onChange={(e) => setName(e.target.value)} placeholder="What contacts will see" className="h-11" autoFocus />

        <label htmlFor="new-pass" className="mt-4 mb-1.5 block text-sm font-medium">Passphrase</label>
        <Input id="new-pass" type="password" value={pass} onChange={(e) => setPass(e.target.value)} autoComplete="off" className="h-11" />
        <div className="mt-2 flex items-center gap-2" aria-live="polite">
          <div className="flex flex-1 gap-1">
            {[0, 1, 2, 3].map((i) => (
              <div key={i} className={cn("h-1.5 flex-1 rounded-full bg-muted", pass && i < score && COLORS[score])} />
            ))}
          </div>
          <span className="w-16 text-right text-xs text-muted-foreground">{pass ? LABELS[score] : ""}</span>
        </div>

        <label htmlFor="again" className="mt-4 mb-1.5 block text-sm font-medium">Repeat passphrase</label>
        <Input id="again" type="password" value={again} onChange={(e) => setAgain(e.target.value)} autoComplete="off" className="h-11" aria-invalid={!!again && again !== pass} />
        {again && again !== pass && <p className="mt-1.5 text-sm text-destructive">The passphrases don't match.</p>}

        <div className="mt-5 flex gap-2 rounded-lg bg-secondary p-3 text-xs text-muted-foreground">
          <ShieldCheck className="mt-0.5 size-4 shrink-0 text-primary" />
          <p>
            This passphrase encrypts everything on your device. <strong className="text-foreground">It cannot be recovered or reset</strong>, so use a few random words you'll remember.
          </p>
        </div>

        {error && <p role="alert" className="mt-3 text-sm text-destructive">{error}</p>}
        <Button type="submit" className="mt-5 h-11 w-full" disabled={!ok || busy}>
          {busy ? <Loader2 className="size-4 animate-spin" /> : null}
          {busy ? "Creating…" : "Create my identity"}
        </Button>
      </form>
      <div className="space-y-2">
        <p className="text-center text-xs text-muted-foreground">New here? Watch the 35 second intro.</p>
        <IntroVideo />
      </div>
     </div>
    </div>
  );
}
