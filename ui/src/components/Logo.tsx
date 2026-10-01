import { cn } from "@/lib/utils";

/**
 * The guft mark: a shield with the name in it. Drawn as strokes, so it needs no
 * font and follows the theme (shield = primary, letters = primary-foreground).
 */
export function Logo({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 128 144" role="img" aria-label="guft" className={cn("h-auto", className)}>
      <path d="M64 5C46 17 26 22 8 22V74C8 106 32 130 64 141C96 130 120 106 120 74V22C102 22 82 17 64 5Z" className="fill-primary" />
      <path
        d="M64 12C48 22 31 27 15 28.5V74C15 101 35 122 64 132C93 122 113 101 113 74V28.5C97 27 80 22 64 12Z"
        className="stroke-primary-foreground/30"
        fill="none"
        strokeWidth="1.6"
      />
      <g transform="translate(0 -1)" fill="none" className="stroke-primary-foreground" strokeWidth="5.6" strokeLinecap="round" strokeLinejoin="round">
        <ellipse cx="31" cy="68" rx="7.5" ry="12" />
        <path d="M38.5 56V82C38.5 89 34.5 93 29 93C27.5 93 26.5 92.8 25.5 92.3" />
        <path d="M49 56V72A7 7 0 0 0 63 72" />
        <path d="M63 56V79" />
        <path d="M75 79V52C75 46.5 78.5 43.5 84.5 43.5" />
        <path d="M68.5 57H82" />
        <path d="M96 47V71C96 76.5 98.5 79.5 103 79.5" />
        <path d="M89.5 57H103" />
      </g>
    </svg>
  );
}
