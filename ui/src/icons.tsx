/**
 * The window's glyphs: 16px strokes on a 16px grid, drawn to sit on the
 * same optical centre as the system font's x-height. Nothing here is an
 * emoji — a glyph at this size is a mark, and an emoji is a cartoon.
 */

import type { ReactNode } from "react";

type IconProps = { className?: string };

/* Every glyph is decorative: the control it sits in carries the name, so a
 * screen reader skips the drawing. */
function Icon({ className, strokeWidth = 1.5, children }: { className: string | undefined; strokeWidth?: number; children: ReactNode }) {
	return (
		<svg
			className={className}
			width={16}
			height={16}
			viewBox="0 0 16 16"
			fill="none"
			stroke="currentColor"
			strokeWidth={strokeWidth}
			strokeLinecap="round"
			strokeLinejoin="round"
			aria-hidden="true"
		>
			{children}
		</svg>
	);
}

export const PlusIcon = ({ className }: IconProps) => (
	<Icon className={className}>
		<path d="M8 3.25v9.5M3.25 8h9.5" />
	</Icon>
);

/* A new teammate: a face, as the rail's rows are, with one more beside it. */
export const PersonPlusIcon = ({ className }: IconProps) => (
	<Icon className={className}>
		<circle cx="6" cy="5" r="2.25" />
		<path d="M2 13c0-2.2 1.8-4 4-4s4 1.8 4 4M12.5 4.5v4M10.5 6.5h4" />
	</Icon>
);

/* The window's own controls, drawn small: a frame's glyphs are marks on
 * the chrome, not buttons in the page. */
export const MinimizeIcon = ({ className }: IconProps) => (
	<Icon className={className}>
		<path d="M4 8.5h8" />
	</Icon>
);

export const MaximizeIcon = ({ className }: IconProps) => (
	<Icon className={className}>
		<rect x="4.5" y="4.5" width="7" height="7" rx="0.75" />
	</Icon>
);

export const RestoreIcon = ({ className }: IconProps) => (
	<Icon className={className}>
		<rect x="4" y="6" width="6" height="6" rx="0.75" />
		<path d="M6.5 6V4.75A.75.75 0 0 1 7.25 4h4a.75.75 0 0 1 .75.75v4a.75.75 0 0 1-.75.75H10" />
	</Icon>
);

export const CloseIcon = ({ className }: IconProps) => (
	<Icon className={className}>
		<path d="M4.5 4.5l7 7M11.5 4.5l-7 7" />
	</Icon>
);

export const TrashIcon = ({ className }: IconProps) => (
	<Icon className={className}>
		<path d="M3 4.5h10M6.5 4.5V3.25h3V4.5M4.5 4.5l.6 8.25h5.8l.6-8.25M6.75 7v3.5M9.25 7v3.5" />
	</Icon>
);

export const SidebarIcon = ({ className }: IconProps) => (
	<Icon className={className}>
		<rect x="2.25" y="3" width="11.5" height="10" rx="2" />
		<path d="M6.25 3v10" />
	</Icon>
);

export const SearchIcon = ({ className }: IconProps) => (
	<Icon className={className}>
		<circle cx="7" cy="7" r="3.75" />
		<path d="M10 10l3 3" />
	</Icon>
);

/* Settings as two sliders: a cog's rays round a circle read as a sun, a light-mode switch. */
export const SettingsIcon = ({ className }: IconProps) => (
	<Icon className={className}>
		<path d="M2.5 5h6M11.5 5h2M2.5 11h2M7.5 11h6" />
		<circle cx="10" cy="5" r="1.5" />
		<circle cx="6" cy="11" r="1.5" />
	</Icon>
);
export const MoreIcon = ({ className }: IconProps) => (
	<Icon className={className}>
		<circle cx="3.5" cy="8" r="1" fill="currentColor" stroke="none" />
		<circle cx="8" cy="8" r="1" fill="currentColor" stroke="none" />
		<circle cx="12.5" cy="8" r="1" fill="currentColor" stroke="none" />
	</Icon>
);

export const InfoIcon = ({ className }: IconProps) => (
	<Icon className={className}>
		<circle cx="8" cy="8" r="5.75" />
		<path d="M8 7.25v3.5" />
		<circle cx="8" cy="5.25" r="0.6" fill="currentColor" stroke="none" />
	</Icon>
);

export const RevealIcon = ({ className }: IconProps) => (
	<Icon className={className}>
		<path d="M6.5 3.5H3.75A1.25 1.25 0 0 0 2.5 4.75v7.5A1.25 1.25 0 0 0 3.75 13.5h7.5a1.25 1.25 0 0 0 1.25-1.25V9.5M9.5 2.5h4v4M13.5 2.5L7.5 8.5" />
	</Icon>
);

export const FolderIcon = ({ className }: IconProps) => (
	<Icon className={className}>
		<path d="M2.5 4.75A1.25 1.25 0 0 1 3.75 3.5h2.4l1.2 1.5h4.9A1.25 1.25 0 0 1 13.5 6.25v5.5a1.25 1.25 0 0 1-1.25 1.25H3.75A1.25 1.25 0 0 1 2.5 11.75z" />
	</Icon>
);

export const FileIcon = ({ className }: IconProps) => (
	<Icon className={className}>
		<path d="M9 2.5H4.75A1.25 1.25 0 0 0 3.5 3.75v8.5a1.25 1.25 0 0 0 1.25 1.25h6.5a1.25 1.25 0 0 0 1.25-1.25V6z" />
		<path d="M9 2.5V6h3.5" />
	</Icon>
);

export const ArrowUpIcon = ({ className }: IconProps) => (
	<Icon className={className} strokeWidth={1.75}>
		<path d="M8 12.5v-9M4.25 7.25L8 3.5l3.75 3.75" />
	</Icon>
);

export const ArrowLeftIcon = ({ className }: IconProps) => (
	<Icon className={className} strokeWidth={1.75}>
		<path d="M12.5 8h-9M7.25 4.25L3.5 8l3.75 3.75" />
	</Icon>
);

export const ArrowDownIcon = ({ className }: IconProps) => (
	<Icon className={className} strokeWidth={1.75}>
		<path d="M8 3.5v9M4.25 8.75L8 12.5l3.75-3.75" />
	</Icon>
);

export const StopIcon = ({ className }: IconProps) => (
	<Icon className={className}>
		<rect x="4.5" y="4.5" width="7" height="7" rx="1.5" fill="currentColor" stroke="none" />
	</Icon>
);

export const ChevronDownIcon = ({ className }: IconProps) => (
	<Icon className={className}>
		<path d="M4.5 6.5L8 10l3.5-3.5" />
	</Icon>
);

export const ChevronRightIcon = ({ className }: IconProps) => (
	<Icon className={className}>
		<path d="M6.5 4.5L10 8l-3.5 3.5" />
	</Icon>
);

export const CheckIcon = ({ className }: IconProps) => (
	<Icon className={className} strokeWidth={1.75}>
		<path d="M3.25 8.5l3 3 6.5-7" />
	</Icon>
);

export const ComputerIcon = ({ className }: IconProps) => (
	<Icon className={className}>
		<rect x="2.25" y="2.75" width="11.5" height="8" rx="1.25" />
		<path d="M6 13.75h4M8 10.75v3" />
	</Icon>
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
		<Icon className={className}>
			<circle cx="8" cy="8" r={r} opacity={0.25} />
			<circle
				className={value === null ? "beat" : undefined}
				cx="8"
				cy="8"
				r={r}
				strokeDasharray={`${shown * around} ${around}`}
				transform="rotate(-90 8 8)"
			/>
		</Icon>
	);
};

export const ClockIcon = ({ className }: IconProps) => (
	<Icon className={className}>
		<circle cx="8" cy="8" r="5.75" />
		<path d="M8 4.75V8l2.25 1.5" />
	</Icon>
);

export const ReplyIcon = ({ className }: IconProps) => (
	<Icon className={className}>
		<path d="M6.5 4L3 7.5 6.5 11M3.25 7.5h5.5A4.25 4.25 0 0 1 13 11.75V12.5" />
	</Icon>
);

export const CopyIcon = ({ className }: IconProps) => (
	<Icon className={className}>
		<rect x="5.25" y="5.25" width="7.5" height="7.5" rx="1.5" />
		<path d="M10.75 3.25H4.75a1.5 1.5 0 0 0-1.5 1.5v6" />
	</Icon>
);

/* A face, for the reaction picker: a mark that stands for the emoji, not one of them. */
export const SmileIcon = ({ className }: IconProps) => (
	<Icon className={className}>
		<circle cx="8" cy="8" r="5.25" />
		<path d="M5.9 9.4a2.6 2.6 0 0 0 4.2 0" />
		<circle cx="6.25" cy="6.6" r="0.6" fill="currentColor" stroke="none" />
		<circle cx="9.75" cy="6.6" r="0.6" fill="currentColor" stroke="none" />
	</Icon>
);

export const WarningIcon = ({ className }: IconProps) => (
	<Icon className={className}>
		<path d="M8 2.75l5.5 9.5h-11z" />
		<path d="M8 6.5v2.75" />
		<circle cx="8" cy="11" r="0.6" fill="currentColor" stroke="none" />
	</Icon>
);

/* Live voice, beside the words in the composer. */
export const VoiceIcon = ({ className }: IconProps) => (
	<Icon className={className}>
		<path d="M2.5 6v4M5.25 3.5v9M8 5v6M10.75 2.5v11M13.5 6v4" />
	</Icon>
);

/* A speech bubble with lines in it, for what was said on a call, written down. */
export const TranscriptIcon = ({ className }: IconProps) => (
	<Icon className={className}>
		<path d="M3 3.25h10a1 1 0 0 1 1 1v6.5a1 1 0 0 1-1 1H8.5L5.5 14v-2.25H3a1 1 0 0 1-1-1v-6.5a1 1 0 0 1 1-1z" />
		<path d="M5 6.25h6M5 8.75h4" />
	</Icon>
);

/* A speaker, for words the teammate said aloud on a call. */
export const SpeakerIcon = ({ className }: IconProps) => (
	<Icon className={className}>
		<path d="M2.75 6.25h2.5L8.5 3.5v9L5.25 9.75h-2.5z" />
		<path d="M11 6.1a2.75 2.75 0 0 1 0 3.8M12.9 4.3a5.3 5.3 0 0 1 0 7.4" />
	</Icon>
);

/* A microphone, for words spoken into the field. */
export const MicIcon = ({ className }: IconProps) => (
	<Icon className={className}>
		<rect x="5.75" y="1.75" width="4.5" height="8" rx="2.25" />
		<path d="M3.5 7.5a4.5 4.5 0 0 0 9 0M8 12v2.25" />
	</Icon>
);

/* A handset, for a call with the desk. */
export const PhoneIcon = ({ className }: IconProps) => (
	<Icon className={className}>
		<path d="M5.2 2.75l1.6 2.9-1.25 1.2a7.6 7.6 0 0 0 3.6 3.6l1.2-1.25 2.9 1.6-.6 2.05a1.5 1.5 0 0 1-1.6 1.05A10.3 10.3 0 0 1 2.1 4.95 1.5 1.5 0 0 1 3.15 3.35z" />
	</Icon>
);

export const PauseIcon = ({ className }: IconProps) => (
	<Icon className={className}>
		<path d="M5.75 4v8M10.25 4v8" />
	</Icon>
);

export const PlayIcon = ({ className }: IconProps) => (
	<Icon className={className}>
		<path d="M5.5 3.9v8.2a.6.6 0 0 0 .9.5l6.4-4.1a.6.6 0 0 0 0-1L6.4 3.4a.6.6 0 0 0-.9.5z" fill="currentColor" stroke="none" />
	</Icon>
);

/** The phone put down: the call glyph turned on its back. */
export const HangUpIcon = ({ className }: IconProps) => (
	<Icon className={className}>
		<path
			transform="rotate(135 8 8)"
			d="M5.2 2.75l1.6 2.9-1.25 1.2a7.6 7.6 0 0 0 3.6 3.6l1.2-1.25 2.9 1.6-.6 2.05a1.5 1.5 0 0 1-1.6 1.05A10.3 10.3 0 0 1 2.1 4.95 1.5 1.5 0 0 1 3.15 3.35z"
		/>
	</Icon>
);

/** The team lead's mark: a star, filled, so it still reads at the size of a dot. */
export const LeadIcon = ({ className }: IconProps) => (
	<Icon className={className} strokeWidth={1}>
		<path fill="currentColor" d="M8 1.75l1.84 3.73 4.11.6-2.97 2.9.7 4.09L8 11.13l-3.68 1.94.7-4.09-2.97-2.9 4.11-.6z" />
	</Icon>
);
