import type { SessionState } from "../generated/contract";
import type { RosterEntry } from "../wire";

/**
 * Each teammate carries a vital sign rather than a status pill: the rail is a
 * roster you watch, so the one moving thing in the whole window is whichever
 * agent is currently working.
 */
const VITAL: Record<SessionState, { color: string; beating: boolean; label: string }> = {
	idle: { color: "var(--rule-strong)", beating: false, label: "idle" },
	starting: { color: "var(--warn)", beating: true, label: "starting" },
	ready: { color: "var(--accent)", beating: false, label: "ready" },
	thinking: { color: "var(--accent)", beating: true, label: "working" },
	error: { color: "var(--danger)", beating: false, label: "error" },
	stopped: { color: "var(--rule-strong)", beating: false, label: "stopped" },
};

export function Rail({
	entries,
	selectedId,
	onSelect,
	onNew,
	onSettings,
}: {
	entries: RosterEntry[];
	selectedId: string | null;
	onSelect(personaId: string): void;
	onNew(): void;
	onSettings(): void;
}) {
	return (
		<nav aria-label="Team" className="flex w-60 shrink-0 flex-col border-r border-rule bg-paper-2">
			<div className="flex items-center justify-between px-3 py-2.5">
				<h1 className="text-xs font-medium uppercase tracking-wider text-ink-3">Team</h1>
				<div className="flex items-center gap-1">
					<button
						type="button"
						className="rounded-md px-1.5 text-sm leading-none text-ink-3 hover:text-ink"
						title="Settings (Ctrl+,)"
						aria-label="Settings"
						onClick={onSettings}
					>
						⚙
					</button>
					<button
						type="button"
						className="rounded-md px-1.5 text-lg leading-none text-ink-3 hover:text-ink"
						title="New teammate (Ctrl+N)"
						aria-label="New teammate"
						onClick={onNew}
					>
						+
					</button>
				</div>
			</div>

			<div className="min-h-0 flex-1 overflow-y-auto px-1.5 pb-2">
				{entries.length === 0 ? (
					<p className="px-2 py-3 text-xs leading-relaxed text-ink-3">
						Add a teammate to get started. Each one keeps its own working directory, its own
						identity, and its own conversation.
					</p>
				) : (
					entries.map((entry, index) => (
						<Row
							key={entry.persona.id}
							entry={entry}
							shortcut={index < 9 ? index + 1 : null}
							active={entry.persona.id === selectedId}
							onSelect={() => onSelect(entry.persona.id)}
						/>
					))
				)}
			</div>
		</nav>
	);
}

function Row({
	entry,
	shortcut,
	active,
	onSelect,
}: {
	entry: RosterEntry;
	shortcut: number | null;
	active: boolean;
	onSelect(): void;
}) {
	const vital = VITAL[entry.session.state];
	const { preview } = entry;
	return (
		<button
			type="button"
			aria-current={active ? "true" : undefined}
			onClick={onSelect}
			className={`flex w-full items-center gap-2.5 rounded-lg px-2 py-1.5 text-left ${
				active ? "bg-paper-4" : "hover:bg-paper-3"
			}`}
		>
			<span
				aria-hidden="true"
				className="grid h-6 w-6 shrink-0 place-items-center rounded-full text-xs font-semibold"
				style={{ background: faceOf(entry.persona.id), color: "oklch(17% 0.004 250)" }}
			>
				{initialOf(entry.persona.name)}
			</span>

			<span className="min-w-0 flex-1">
				<span className="flex items-center gap-1.5">
					<span className={`truncate font-medium ${active ? "text-ink" : "text-ink-2"}`}>
						{entry.persona.name}
					</span>
					<span
						aria-hidden="true"
						className={`ml-auto h-2 w-2 shrink-0 rounded-full ${vital.beating ? "animate-throat" : ""}`}
						style={{ background: vital.color }}
					/>
					<span className="sr-only">{vital.label}</span>
				</span>
				<span className="block truncate text-xs text-ink-3">
					{preview ? `${preview.from === "me" ? "you: " : ""}${oneLine(preview.text)}` : vital.label}
				</span>
			</span>

			{shortcut !== null && (
				<span aria-hidden="true" className="shrink-0 font-mono text-[0.6875rem] text-ink-3">
					⌃{shortcut}
				</span>
			)}
		</button>
	);
}

/**
 * A preview is one line in a narrow rail, whatever shape it was written in.
 * The markdown an agent writes is rendered in the bubble and stripped here:
 * asterisks and backticks in a two-inch column are noise, not emphasis.
 */
function oneLine(source: string): string {
	return source
		.replace(/^\s{0,3}(```|~~~).*$/gm, "") // fence delimiters, keeping the body
		.replace(/^\s{0,3}#{1,6}\s+/gm, "") // heading hashes
		.replace(/^\s{0,3}>\s?/gm, "") // quote marks
		.replace(/^\s*([-*+]|\d{1,9}[.)])\s+/gm, "") // list markers
		.replace(/!?\[([^\]]*)\]\([^)]*\)/g, "$1") // links and images, keeping the text
		.replace(/[*_~`]/g, "") // emphasis and code marks
		.replace(/\s+/g, " ")
		.trim();
}

/**
 * A face colour from the id rather than from the roster position, so a
 * teammate does not change colour when the one above it is deleted. Red is
 * missing on purpose: it is the colour of something being wrong.
 */
function faceOf(personaId: string): string {
	let hash = 0;
	for (let index = 0; index < personaId.length; index++) {
		hash = (hash * 31 + personaId.charCodeAt(index)) % 1_000_003;
	}
	return `oklch(72% 0.13 ${70 + (hash % 7) * 43})`;
}

/** The first letter that is one, so "⌘kill bill" and " Ada" both read right. */
function initialOf(name: string): string {
	return (name.match(/\p{L}|\p{N}/u)?.[0] ?? "?").toUpperCase();
}
