/**
 * The window's glyphs. Small inline SVGs, the same names the previous Toad
 * used, so a button that was a gear stays a gear. Nothing here is an emoji:
 * a glyph at this size is a mark, and an emoji at this size is a cartoon.
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

export const CloseIcon = ({ className }: IconProps) => (
	<svg className={className} {...box}>
		<path d="M4 4l8 8M12 4l-8 8" />
	</svg>
);

export const SearchIcon = ({ className }: IconProps) => (
	<svg className={className} {...box}>
		<circle cx="7" cy="7" r="3.75" />
		<path d="M10 10l3 3" />
	</svg>
);

export const CogIcon = ({ className }: IconProps) => (
	<svg className={className} {...box}>
		<circle cx="8" cy="8" r="2.25" />
		<path d="M8 2.5v1.25M8 12.25V13.5M2.5 8h1.25M12.25 8H13.5M4.05 4.05l.88.88M11.07 11.07l.88.88M4.05 11.95l.88-.88M11.07 4.93l.88-.88" />
	</svg>
);

export const MoreIcon = ({ className }: IconProps) => (
	<svg className={className} {...box}>
		<circle cx="4" cy="8" r="0.9" fill="currentColor" stroke="none" />
		<circle cx="8" cy="8" r="0.9" fill="currentColor" stroke="none" />
		<circle cx="12" cy="8" r="0.9" fill="currentColor" stroke="none" />
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
