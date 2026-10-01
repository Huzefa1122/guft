import { Logo } from "./Logo";

export function Brand({ size = "md" }: { size?: "md" | "lg" }) {
  return (
    <div className="flex items-center gap-2.5">
      <Logo className={size === "lg" ? "w-12" : "w-7"} />
      <span className={size === "lg" ? "text-2xl font-semibold tracking-tight" : "text-lg font-semibold tracking-tight"}>guft</span>
    </div>
  );
}
