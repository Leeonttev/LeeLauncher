import * as React from "react";
import { cn } from "cn";
import { Slider as SliderPrimitive } from "radix-ui";

function Slider({
	className,
	defaultValue,
	value,
	min = 0,
	max = 100,
	thumbProps,
	...props
}: React.ComponentProps<typeof SliderPrimitive.Root> & {
	thumbProps?: React.ComponentProps<typeof SliderPrimitive.Thumb>;
}) {
	const _values = React.useMemo(
		() => (Array.isArray(value) ? value : Array.isArray(defaultValue) ? defaultValue : [min, max]),
		[value, defaultValue, min, max],
	);

	return (
		<SliderPrimitive.Root
			data-slot="slider"
			defaultValue={defaultValue}
			value={value}
			min={min}
			max={max}
			className={cn(
				"relative flex w-full touch-none items-center select-none data-[disabled]:opacity-50 data-[orientation=vertical]:h-full data-[orientation=vertical]:min-h-44 data-[orientation=vertical]:w-auto data-[orientation=vertical]:flex-col",
				className,
			)}
			{...props}
		>
			<SliderPrimitive.Track
				data-slot="slider-track"
				className={cn(
					"relative grow overflow-clip rounded-full bg-border data-[orientation=horizontal]:h-1 data-[orientation=horizontal]:w-full data-[orientation=vertical]:h-full data-[orientation=vertical]:w-1",
				)}
			>
				<SliderPrimitive.Range
					data-slot="slider-range"
					className={cn(
						"absolute rounded-full bg-primary data-[orientation=horizontal]:h-full data-[orientation=vertical]:w-full",
					)}
				/>
			</SliderPrimitive.Track>
			{Array.from({ length: _values.length }, (_, index) => (
				<SliderPrimitive.Thumb
					data-slot="slider-thumb"
					key={index}
					{...thumbProps}
					className="relative block size-[18px] shrink-0 cursor-grab rounded-full border border-control-track bg-white transition-colors duration-(--duration-fast) ease-out active:cursor-grabbing disabled:pointer-events-none disabled:opacity-50"
				/>
			))}
		</SliderPrimitive.Root>
	);
}

export { Slider };
