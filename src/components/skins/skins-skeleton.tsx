import { Skeleton } from "@/components/ui/skeleton";
import { useDelayedFlag } from "@/hooks/use-delayed-flag";

export function SkinCardSkeleton() {
	return (
		<div className="relative w-38 rounded-xl border border-border px-3 pt-3 pb-2.5">
			<Skeleton className="mx-auto h-32 w-16 rounded-lg" />
			<Skeleton className="mt-3 h-4 w-20" />
			<Skeleton className="mt-2 h-3 w-24" />
		</div>
	);
}

export function SkinsPageSkeleton() {
	const late = useDelayedFlag(true, 150);
	if (!late) return <div className="flex-1" />;
	return (
		<div aria-busy className="@container/skins flex min-h-0 flex-1">
			<div className="flex w-85 shrink-0 flex-col border-r @min-[1100px]/skins:w-100">
				<Skeleton className="min-h-0 flex-1" />
				<div className="h-40 border-t" />
			</div>
			<div className="flex min-w-0 flex-1 flex-col overflow-hidden px-8 pt-7">
				<Skeleton className="h-9 w-32" />
				<div className="mt-5 grid grid-cols-[repeat(auto-fill,152px)] gap-3">
					{Array.from({ length: 12 }, (_, index) => (
						<SkinCardSkeleton key={index} />
					))}
				</div>
			</div>
		</div>
	);
}
