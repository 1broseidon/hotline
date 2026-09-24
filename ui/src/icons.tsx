/**
 * The window's glyphs: 16px strokes on a 16px grid, drawn to sit on the
 * same optical centre as the system font's x-height. Nothing here is an
 * emoji — a glyph at this size is a mark, and an emoji is a cartoon.
 */

type IconProps = { className?: string };

const box = {
	width: 16,
	height: 16,
	viewBox: "0 0 16 16",
	fill: "none",
	stroke: "currentColor",
	strokeWidth: 1.5,
	strokeLinecap: "round" as const,
	strokeLinejoin: "round" as const,
	"aria-hidden": true as const,
};

export const PlusIcon = ({ className }: IconProps) => (
	<svg className={className} {...box}>
		<path d="M8 3.25v9.5M3.25 8h9.5" />
	</svg>
);

/* The window's own controls, drawn small: a frame's glyphs are marks on
 * the chrome, not buttons in the page. */
export const MinimizeIcon = ({ className }: IconProps) => (
	<svg className={className} {...box}>
		<path d="M4 8.5h8" />
	</svg>
);

export const MaximizeIcon = ({ className }: IconProps) => (
	<svg className={className} {...box}>
		<rect x="4.5" y="4.5" width="7" height="7" rx="0.75" />
	</svg>
);

export const RestoreIcon = ({ className }: IconProps) => (
	<svg className={className} {...box}>
		<rect x="4" y="6" width="6" height="6" rx="0.75" />
		<path d="M6.5 6V4.75A.75.75 0 0 1 7.25 4h4a.75.75 0 0 1 .75.75v4a.75.75 0 0 1-.75.75H10" />
	</svg>
);

export const CloseIcon = ({ className }: IconProps) => (
	<svg className={className} {...box}>
		<path d="M4.5 4.5l7 7M11.5 4.5l-7 7" />
	</svg>
);

export const SearchIcon = ({ className }: IconProps) => (
	<svg className={className} {...box}>
		<circle cx="7" cy="7" r="3.75" />
		<path d="M10 10l3 3" />
	</svg>
);

export const GearIcon = ({ className }: IconProps) => (
	<svg className={className} {...box}>
		<circle cx="8" cy="8" r="2.25" />
		<path d="M8 2.5v1.25M8 12.25V13.5M2.5 8h1.25M12.25 8H13.5M4.05 4.05l.88.88M11.07 11.07l.88.88M4.05 11.95l.88-.88M11.07 4.93l.88-.88" />
	</svg>
);

export const MoreIcon = ({ className }: IconProps) => (
	<svg className={className} {...box}>
		<circle cx="3.5" cy="8" r="1" fill="currentColor" stroke="none" />
		<circle cx="8" cy="8" r="1" fill="currentColor" stroke="none" />
		<circle cx="12.5" cy="8" r="1" fill="currentColor" stroke="none" />
	</svg>
);

export const InfoIcon = ({ className }: IconProps) => (
	<svg className={className} {...box}>
		<circle cx="8" cy="8" r="5.75" />
		<path d="M8 7.25v3.5" />
		<circle cx="8" cy="5.25" r="0.6" fill="currentColor" stroke="none" />
	</svg>
);

export const RevealIcon = ({ className }: IconProps) => (
	<svg className={className} {...box}>
		<path d="M6.5 3.5H3.75A1.25 1.25 0 0 0 2.5 4.75v7.5A1.25 1.25 0 0 0 3.75 13.5h7.5a1.25 1.25 0 0 0 1.25-1.25V9.5M9.5 2.5h4v4M13.5 2.5L7.5 8.5" />
	</svg>
);

export const FolderIcon = ({ className }: IconProps) => (
	<svg className={className} {...box}>
		<path d="M2.5 4.75A1.25 1.25 0 0 1 3.75 3.5h2.4l1.2 1.5h4.9A1.25 1.25 0 0 1 13.5 6.25v5.5a1.25 1.25 0 0 1-1.25 1.25H3.75A1.25 1.25 0 0 1 2.5 11.75z" />
	</svg>
);

export const ArrowUpIcon = ({ className }: IconProps) => (
	<svg className={className} {...box} strokeWidth={1.75}>
		<path d="M8 12.5v-9M4.25 7.25L8 3.5l3.75 3.75" />
	</svg>
);

export const ArrowLeftIcon = ({ className }: IconProps) => (
	<svg className={className} {...box} strokeWidth={1.75}>
		<path d="M12.5 8h-9M7.25 4.25L3.5 8l3.75 3.75" />
	</svg>
);

export const ArrowDownIcon = ({ className }: IconProps) => (
	<svg className={className} {...box} strokeWidth={1.75}>
		<path d="M8 3.5v9M4.25 8.75L8 12.5l3.75-3.75" />
	</svg>
);

export const StopIcon = ({ className }: IconProps) => (
	<svg className={className} {...box}>
		<rect x="4.5" y="4.5" width="7" height="7" rx="1.5" fill="currentColor" stroke="none" />
	</svg>
);

export const ChevronDownIcon = ({ className }: IconProps) => (
	<svg className={className} {...box}>
		<path d="M4.5 6.5L8 10l3.5-3.5" />
	</svg>
);

export const ChevronRightIcon = ({ className }: IconProps) => (
	<svg className={className} {...box}>
		<path d="M6.5 4.5L10 8l-3.5 3.5" />
	</svg>
);

export const CheckIcon = ({ className }: IconProps) => (
	<svg className={className} {...box} strokeWidth={1.75}>
		<path d="M3.25 8.5l3 3 6.5-7" />
	</svg>
);

export const ComputerIcon = ({ className }: IconProps) => (
	<svg className={className} {...box}>
		<rect x="2.25" y="2.75" width="11.5" height="8" rx="1.25" />
		<path d="M6 13.75h4M8 10.75v3" />
	</svg>
);

/**
 * A download filling in, where the computer's glyph will stand once it is
 * done: a faint ring and the share that has landed. With no count yet, a
 * short arc breathes like a working teammate's mark.
 */
export const ProgressRing = ({ className, value }: IconProps & { value: number | null }) => {
	const r = 5.75;
	const around = 2 * Math.PI * r;
	const shown = value === null ? 0.25 : Math.min(1, Math.max(0.04, value));
	return (
		<svg className={className} {...box}>
			<circle cx="8" cy="8" r={r} opacity={0.25} />
			<circle
				className={value === null ? "beat" : undefined}
				cx="8"
				cy="8"
				r={r}
				strokeDasharray={`${shown * around} ${around}`}
				transform="rotate(-90 8 8)"
			/>
		</svg>
	);
};

export const ClockIcon = ({ className }: IconProps) => (
	<svg className={className} {...box}>
		<circle cx="8" cy="8" r="5.75" />
		<path d="M8 4.75V8l2.25 1.5" />
	</svg>
);

export const BookIcon = ({ className }: IconProps) => (
	<svg className={className} {...box}>
		<path d="M3 3.25h4.25A1.5 1.5 0 0 1 8.75 4.75V13a1.5 1.5 0 0 0-1.5-1.5H3zM13 3.25H8.75A1.5 1.5 0 0 0 7.25 4.75V13a1.5 1.5 0 0 1 1.5-1.5H13z" />
	</svg>
);

export const ReplyIcon = ({ className }: IconProps) => (
	<svg className={className} {...box}>
		<path d="M6.5 4L3 7.5 6.5 11M3.25 7.5h5.5A4.25 4.25 0 0 1 13 11.75V12.5" />
	</svg>
);

export const WarningIcon = ({ className }: IconProps) => (
	<svg className={className} {...box}>
		<path d="M8 2.75l5.5 9.5h-11z" />
		<path d="M8 6.5v2.75" />
		<circle cx="8" cy="11" r="0.6" fill="currentColor" stroke="none" />
	</svg>
);
