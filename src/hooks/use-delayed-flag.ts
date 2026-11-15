import { useEffect, useState } from "react";

/** `true`, если `flag` продержался дольше `delay` мс: скелетон не мигает на быстрых ответах. */
export function useDelayedFlag(flag: boolean, delay: number) {
	const [late, setLate] = useState(false);
	useEffect(() => {
		if (!flag) return;
		const timer = setTimeout(() => setLate(true), delay);
		return () => {
			clearTimeout(timer);
			setLate(false);
		};
	}, [flag, delay]);
	return flag && late;
}
