import { useEffect, useRef, useState, type FormEvent } from "react";
import { Eye, EyeOff, KeyRound, Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { errorText } from "@/lib/api";
import { useApp } from "@/lib/store";
import { Brand } from "./Brand";

export function LockScreen() {
  const { unlock, status } = useApp();
  const [pass, setPass] = useState("");
  const [show, setShow] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const input = useRef<HTMLInputElement>(null);

  useEffect(() => input.current?.focus(), []);

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!pass || busy) return;
    setBusy(true);
    setError("");
    try {
      await unlock(pass);
      setPass("");
    } catch (err) {
      setError(errorText(err));
      setPass("");
      input.current?.focus();
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="grid h-full place-items-center bg-background p-6">
      <form onSubmit={submit} className="w-full max-w-sm rounded-2xl border bg-card p-8 shadow-sm">
        <div className="mb-6 flex flex-col items-center gap-3 text-center">
          <Brand size="lg" />
          <p className="text-sm text-muted-foreground">
            {status?.name ? `Welcome back, ${status.name}.` : "Welcome back."} Your messages are locked.
          </p>
        </div>
        <label htmlFor="pass" className="mb-1.5 block text-sm font-medium">
          Passphrase
        </label>
        <div className="relative">
          <KeyRound className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            id="pass"
            ref={input}
            type={show ? "text" : "password"}
            value={pass}
            onChange={(e) => setPass(e.target.value)}
            autoComplete="off"
            spellCheck={false}
            className="h-11 pr-10 pl-9"
            aria-invalid={!!error}
            aria-describedby={error ? "pass-error" : undefined}
          />
          <button
            type="button"
            onClick={() => setShow((s) => !s)}
            className="absolute top-1/2 right-3 -translate-y-1/2 text-muted-foreground hover:text-foreground"
            aria-label={show ? "Hide passphrase" : "Show passphrase"}
          >
            {show ? <EyeOff className="size-4" /> : <Eye className="size-4" />}
          </button>
        </div>
        {error && (
          <p id="pass-error" role="alert" className="mt-2 text-sm text-destructive">
            {error}
          </p>
        )}
        <Button type="submit" className="mt-5 h-11 w-full" disabled={!pass || busy}>
          {busy ? <Loader2 className="size-4 animate-spin" /> : null}
          {busy ? "Unlocking…" : "Unlock"}
        </Button>
        <p className="mt-4 text-center text-xs text-muted-foreground">
          Everything stays encrypted on this device. There is no account and no recovery: only your passphrase opens it.
        </p>
      </form>
    </div>
  );
}
