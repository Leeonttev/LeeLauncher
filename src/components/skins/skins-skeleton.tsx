import { Skeleton } from "@/components/ui/skeleton";
import { useDelayedFlag } from "@/hooks/use-delayed-flag";

export function SkinCardSkeleton() {
	return (
		<div className="relative h-54 w-38 rounded-xl border border-border px-3 pt-8 pb-2.5">
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
		<div aria-busy className="@container/skins flex h-full min-h-0 flex-1 gap-8 px-8 py-6">
			<div className="flex w-80 shrink-0 flex-col gap-4 @min-[1280px]/skins:w-[380px]">
				<Skeleton className="min-h-0 flex-1 rounded-2xl" />
				<Skeleton className="h-8 w-full rounded-lg" />
				<Skeleton className="h-10 w-full rounded-full" />
			</div>
			<div className="flex min-w-0 flex-1 flex-col overflow-hidden">
				<Skeleton className="mt-2 h-7 w-36" />
				<div className="mt-5 grid grid-cols-[repeat(auto-fill,152px)] gap-3">
					{Array.from({ length: 12 }, (_, index) => (
						<SkinCardSkeleton key={index} />
					))}
				</div>
			</div>
		</div>
	);
}
