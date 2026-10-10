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

export const TrashIcon = ({ className }: IconProps) => (
	<svg className={className} {...box}>
		<path d="M3 4.5h10M6.5 4.5V3.25h3V4.5M4.5 4.5l.6 8.25h5.8l.6-8.25M6.75 7v3.5M9.25 7v3.5" />
	</svg>
);

export const SidebarIcon = ({ className }: IconProps) => (
	<svg className={className} {...box}>
		<rect x="2.25" y="3" width="11.5" height="10" rx="2" />
		<path d="M6.25 3v10" />
	</svg>
);

export const SearchIcon = ({ className }: IconProps) => (
	<svg className={className} {...box}>
		<circle cx="7" cy="7" r="3.75" />
		<path d="M10 10l3 3" />
	</svg>
);

/* Settings as two sliders: a cog's rays round a circle read as a sun, a light-mode switch. */
export const SettingsIcon = ({ className }: IconProps) => (
	<svg className={className} {...box}>
		<path d="M2.5 5h6M11.5 5h2M2.5 11h2M7.5 11h6" />
		<circle cx="10" cy="5" r="1.5" />
		<circle cx="6" cy="11" r="1.5" />
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

export const FileIcon = ({ className }: IconProps) => (
	<svg className={className} {...box}>
		<path d="M9 2.5H4.75A1.25 1.25 0 0 0 3.5 3.75v8.5a1.25 1.25 0 0 0 1.25 1.25h6.5a1.25 1.25 0 0 0 1.25-1.25V6z" />
		<path d="M9 2.5V6h3.5" />
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

export const CopyIcon = ({ className }: IconProps) => (
	<svg className={className} {...box}>
		<rect x="5.25" y="5.25" width="7.5" height="7.5" rx="1.5" />
		<path d="M10.75 3.25H4.75a1.5 1.5 0 0 0-1.5 1.5v6" />
	</svg>
);

/* A face, for the reaction picker: a mark that stands for the emoji, not one of them. */
export const SmileIcon = ({ className }: IconProps) => (
	<svg className={className} {...box}>
		<circle cx="8" cy="8" r="5.25" />
		<path d="M5.9 9.4a2.6 2.6 0 0 0 4.2 0" />
		<circle cx="6.25" cy="6.6" r="0.6" fill="currentColor" stroke="none" />
		<circle cx="9.75" cy="6.6" r="0.6" fill="currentColor" stroke="none" />
	</svg>
);

export const WarningIcon = ({ className }: IconProps) => (
	<svg className={className} {...box}>
		<path d="M8 2.75l5.5 9.5h-11z" />
		<path d="M8 6.5v2.75" />
		<circle cx="8" cy="11" r="0.6" fill="currentColor" stroke="none" />
	</svg>
);

/* Live voice, beside the words in the composer. */
export const VoiceIcon = ({ className }: IconProps) => (
	<svg className={className} {...box}>
		<path d="M2.5 6v4M5.25 3.5v9M8 5v6M10.75 2.5v11M13.5 6v4" />
	</svg>
);

/* A speaker, for words the teammate said aloud on a call. */
export const SpeakerIcon = ({ className }: IconProps) => (
	<svg className={className} {...box}>
		<path d="M2.75 6.25h2.5L8.5 3.5v9L5.25 9.75h-2.5z" />
		<path d="M11 6.1a2.75 2.75 0 0 1 0 3.8M12.9 4.3a5.3 5.3 0 0 1 0 7.4" />
	</svg>
);

/* A microphone, for words spoken into the field. */
export const MicIcon = ({ className }: IconProps) => (
	<svg className={className} {...box}>
		<rect x="5.75" y="1.75" width="4.5" height="8" rx="2.25" />
		<path d="M3.5 7.5a4.5 4.5 0 0 0 9 0M8 12v2.25" />
	</svg>
);

/* A handset, for a call with the desk. */
export const PhoneIcon = ({ className }: IconProps) => (
	<svg className={className} {...box}>
		<path d="M5.2 2.75l1.6 2.9-1.25 1.2a7.6 7.6 0 0 0 3.6 3.6l1.2-1.25 2.9 1.6-.6 2.05a1.5 1.5 0 0 1-1.6 1.05A10.3 10.3 0 0 1 2.1 4.95 1.5 1.5 0 0 1 3.15 3.35z" />
	</svg>
);

export const PauseIcon = ({ className }: IconProps) => (
	<svg className={className} {...box}>
		<path d="M5.75 4v8M10.25 4v8" />
	</svg>
);

export const PlayIcon = ({ className }: IconProps) => (
	<svg className={className} {...box}>
		<path d="M5.5 3.9v8.2a.6.6 0 0 0 .9.5l6.4-4.1a.6.6 0 0 0 0-1L6.4 3.4a.6.6 0 0 0-.9.5z" fill="currentColor" stroke="none" />
	</svg>
);

/** The phone put down: the call glyph turned on its back. */
export const HangUpIcon = ({ className }: IconProps) => (
	<svg className={className} {...box}>
		<path
			transform="rotate(135 8 8)"
			d="M5.2 2.75l1.6 2.9-1.25 1.2a7.6 7.6 0 0 0 3.6 3.6l1.2-1.25 2.9 1.6-.6 2.05a1.5 1.5 0 0 1-1.6 1.05A10.3 10.3 0 0 1 2.1 4.95 1.5 1.5 0 0 1 3.15 3.35z"
		/>
	</svg>
);
