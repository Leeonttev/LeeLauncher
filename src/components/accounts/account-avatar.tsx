import { User } from "lucide-react";
import { useEffect, useState } from "react";
import type { Account, WornSkin } from "@/lib/api";
import { cachedSkinHead, skinHead } from "@/lib/skin-texture";
import { useWornSkins } from "@/lib/skins";
import { cn } from "@/lib/utils";

/** Голова 8×8: размеры кратны 8, чтобы пиксель головы был целым. 16 — только в подписях запущенной игры. */
export type AvatarSize = 16 | 24 | 32 | 40;

const SIZE: Record<AvatarSize, string> = {
	16: "size-4 rounded-[4px]",
	24: "size-6 rounded-sm",
	32: "size-8 rounded-sm",
	40: "size-10 rounded-md",
};

/** Сервис увеличивает голову без сглаживания, поэтому просим ровно нужный размер. */
function headPx(size: AvatarSize) {
	return size;
}

/** Microsoft — по uuid, офлайн — по нику; неизвестный ник сервис отдаёт головой Стива. */
function headUrl(account: Pick<Account, "kind" | "username" | "uuid">, size: AvatarSize) {
	const id =
		account.kind === "microsoft"
			? account.uuid.replaceAll("-", "")
			: encodeURIComponent(account.username);
	return `https://mc-heads.net/avatar/${id}/${headPx(size)}`;
}

type AccountAvatarProps = {
	account: Pick<Account, "id" | "kind" | "username" | "uuid" | "needsRelogin">;
	size: AvatarSize;
	surface?: "sidebar" | "popover" | "cover";
	/** Красная точка «нужно войти снова». */
	showStatus?: boolean;
	className?: string;
};

/** Пока неизвестно, есть ли сохранённый скин, — скелетон: иначе мелькнёт чужая голова с mc-heads.net. */
export function AccountAvatar({
	account,
	size,
	surface = "sidebar",
	showStatus = true,
	className,
}: AccountAvatarProps) {
	const worn = useWornSkins();
	const microsoft = account.kind === "microsoft";
	const record = microsoft ? worn.map[account.id] : undefined;
	const remote = <RemoteHead account={account} size={size} />;
	return (
		<span className={cn("relative inline-flex shrink-0", className)}>
			{microsoft && worn.status === "loading" ? (
				<HeadFrame size={size} pulse />
			) : record ? (
				<WornHead key={record.key} worn={record} size={size} fallback={remote} />
			) : (
				remote
			)}
			{showStatus && account.needsRelogin && (
				<span
					aria-hidden
					className={cn(
						"absolute -right-1 -bottom-1 flex size-3 items-center justify-center rounded-full",
						surface === "sidebar"
							? "bg-sidebar"
							: surface === "popover"
								? "bg-popover"
								: "bg-cover-scrim",
					)}
				>
					<span className="size-2 rounded-full bg-destructive" />
				</span>
			)}
		</span>
	);
}

function RemoteHead({ account, size }: Pick<AccountAvatarProps, "account" | "size">) {
	const src = headUrl(account, size);
	return (
		<Head
			key={src}
			src={src}
			fallbackSrc={`https://mc-heads.net/avatar/MHF_Steve/${headPx(size)}`}
			size={size}
		/>
	);
}

function HeadFrame({
	size,
	pulse,
	children,
}: {
	size: AvatarSize;
	pulse?: boolean;
	children?: React.ReactNode;
}) {
	return (
		<span
			aria-hidden
			className={cn(
				"relative flex items-center justify-center overflow-clip bg-foreground/10 select-none",
				SIZE[size],
				pulse && "animate-pulse",
			)}
		>
			{children}
		</span>
	);
}

/** Голова из сохранённого скина; не вышло — голова с mc-heads.net. */
function WornHead({ worn, size, fallback }: { worn: WornSkin; size: AvatarSize; fallback: React.ReactNode }) {
	const [drawn, setDrawn] = useState<string | null>(() => cachedSkinHead(worn.key));
	const [failed, setFailed] = useState(false);

	useEffect(() => {
		const texture = worn.texture;
		if (drawn || !texture) return;
		let stale = false;
		skinHead(worn.key, texture).then(
			(url) => {
				if (!stale) setDrawn(url);
			},
			(err: unknown) => {
				console.error("Не удалось нарисовать голову из скина", err);
				if (!stale) setFailed(true);
			},
		);
		return () => {
			stale = true;
		};
	}, [drawn, worn.key, worn.texture]);

	if (failed || !worn.texture) return fallback;
	return (
		<HeadFrame size={size} pulse={!drawn}>
			{drawn && (
				<img
					src={drawn}
					alt=""
					draggable={false}
					onError={() => setFailed(true)}
					className="absolute inset-0 size-full [image-rendering:pixelated]"
				/>
			)}
		</HeadFrame>
	);
}

function Head({ src, fallbackSrc, size }: { src: string; fallbackSrc: string; size: AvatarSize }) {
	const [current, setCurrent] = useState(src);
	const [state, setState] = useState<"loading" | "loaded" | "error">("loading");
	return (
		<HeadFrame size={size} pulse={state === "loading"}>
			{state === "error" ? (
				<User className="text-muted-foreground" />
			) : (
				<img
					src={current}
					alt=""
					draggable={false}
					referrerPolicy="no-referrer"
					onLoad={() => setState("loaded")}
					onError={() => {
						if (current !== fallbackSrc) setCurrent(fallbackSrc);
						else setState("error");
					}}
					className={cn(
						"absolute inset-0 size-full transition-opacity duration-(--duration-base) ease-out [image-rendering:pixelated]",
						state === "loaded" ? "opacity-100" : "opacity-0",
					)}
				/>
			)}
		</HeadFrame>
	);
}
