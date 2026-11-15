/** Кольцо фокуса пункта — рамка внутри на `::after`: снаружи до рамки группы всего 3 px, а `outline` внутрь закрыла бы «таблетка». */
export const segmentGroup =
	"inline-flex shrink-0 items-center gap-0.5 rounded-lg bg-muted p-[3px] ring-1 ring-border ring-inset";

export const segmentItem =
	"relative flex h-7 cursor-pointer items-center justify-center rounded-[9px] text-[13px]/[18px] font-medium text-muted-foreground transition-colors duration-(--duration-fast) ease-out hover:text-foreground after:pointer-events-none after:absolute after:inset-0 after:rounded-[9px] after:border-2 after:border-transparent focus-visible:outline-none focus-visible:after:border-ring disabled:pointer-events-none disabled:opacity-50 aria-checked:text-foreground";

export const segmentThumb =
	"absolute inset-0 rounded-[9px] bg-card ring-1 ring-border dark:bg-accent dark:ring-0";

export const segmentSpring = { type: "spring", stiffness: 500, damping: 40, mass: 0.8 } as const;
