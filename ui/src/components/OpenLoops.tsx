import { useEffect, useState } from "react";
import type { OpenLoop } from "../generated/contract";
import { CloseIcon } from "../icons";
import { wire } from "../wire";

const MINUTE = 60_000;
const HOUR = 3_600_000;
const DAY = 86_400_000;

const DATE_FMT = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric" });

function ageText(updatedAt: number, now: number): string {
	const diff = Math.max(0, now - updatedAt);
	if (diff < MINUTE) return "Just now";
	if (diff < HOUR) return `${Math.floor(diff / MINUTE)}m ago`;
	if (diff < 2 * DAY) return `${Math.floor(diff / HOUR)}h ago`;
	return `${Math.floor(diff / DAY)}d ago`;
}

function stateLabel(state: OpenLoop["state"]): string {
	switch (state) {
		case "active":
			return "Active";
		case "waiting":
			return "Waiting";
		case "stale":
			return "Stale";
		case "done":
			return "Done";
		case "archived":
			return "Archived";
	}
}

function timingLabel(loop: OpenLoop, now: number): string {
	if (loop.reviewBy !== undefined) {
		const date = DATE_FMT.format(new Date(loop.reviewBy));
		return loop.state === "active" ? `Review ${date}` : `Due ${date}`;
	}
	return ageText(loop.updatedAt, now);
}

/**
 * Living open loops carried by a teammate across chapters.
 *
 * Subscribes live to `{"loops": personaId}`, displays active/waiting/stale rows
 * above Schedules with one-tap Ask and Dismiss, and keeps done/archived items
 * tucked behind a collapsed disclosure.
 */
export function OpenLoops({
	personaId,
	onAskLoop,
}: {
	personaId: string;
	onAskLoop?: ((text: string) => void) | undefined;
}) {
	const [loops, setLoops] = useState<OpenLoop[]>([]);
	const [showSettled, setShowSettled] = useState(false);

	useEffect(() => {
		setLoops([]);
		setShowSettled(false);
		return wire.subscribe<OpenLoop>(
			{ loops: personaId },
			{
				snapshot: (entries) => setLoops(entries),
				event: (entry) =>
					setLoops((known) => {
						const at = known.findIndex((item) => item.id === entry.id);
						if (at === -1) return [entry, ...known];
						const next = known.slice();
						next[at] = entry;
						return next;
					}),
				removed: () => setLoops([]),
			},
		);
	}, [personaId]);

	const open = loops.filter((loop) => loop.state === "stale" || loop.state === "active" || loop.state === "waiting");
	const settled = loops.filter((loop) => loop.state === "done" || loop.state === "archived");

	if (open.length === 0 && settled.length === 0) return null;

	const now = Date.now();

	return (
		<section>
			<h3 className="label">Open loops</h3>
			{open.length > 0 ? (
				<div className="grouped">
					{open.map((loop) => (
						<LoopRow key={loop.id} personaId={personaId} loop={loop} now={now} onAskLoop={onAskLoop} />
					))}
				</div>
			) : (
				<p className="group-hint">No open loops.</p>
			)}
			{settled.length > 0 && (
				<div className="mt-2">
					<button
						type="button"
						className="control btn-quiet btn-sm -ml-2"
						aria-expanded={showSettled}
						onClick={() => setShowSettled((was) => !was)}
					>
						{showSettled ? "Hide" : "Completed & archived"} ({settled.length})
					</button>
					{showSettled && (
						<div className="grouped mt-1.5">
							{settled.map((loop) => (
								<div key={loop.id} className="group-row items-start gap-2 py-2 opacity-75">
									<span className="group-row-text">
										<span className="group-row-title" title={loop.summary}>
											{loop.summary}
										</span>
										<span className="group-row-detail">
											{stateLabel(loop.state)} · {DATE_FMT.format(new Date(loop.closedAt ?? loop.updatedAt))}
										</span>
									</span>
								</div>
							))}
						</div>
					)}
				</div>
			)}
		</section>
	);
}

function LoopRow({
	personaId,
	loop,
	now,
	onAskLoop,
}: {
	personaId: string;
	loop: OpenLoop;
	now: number;
	onAskLoop?: ((text: string) => void) | undefined;
}) {
	const [busy, setBusy] = useState(false);
	const [said, setSaid] = useState<string | null>(null);

	const dismiss = async () => {
		if (busy) return;
		setBusy(true);
		setSaid(null);
		try {
			await wire.command("loops.dismiss", { personaId, id: loop.id });
		} catch (error) {
			setSaid(error instanceof Error ? error.message : String(error));
			setBusy(false);
		}
	};

	return (
		<div className="group-row items-start gap-2 py-2">
			<span className="group-row-text">
				<span className="group-row-title" title={loop.summary}>
					{loop.summary}
				</span>
				<span className="group-row-detail schedule-detail">
					<span className="schedule-detail-parts">
						<span style={loop.state === "stale" ? { color: "var(--warn-ink)" } : undefined}>{stateLabel(loop.state)}</span>
						<span>
							<span className="sr-only"> · </span>
							{timingLabel(loop, now)}
						</span>
						{loop.jobId !== undefined && (
							<span>
								<span className="sr-only"> · </span>
								Tracks job
							</span>
						)}
					</span>
				</span>
				{said !== null && <span className="mt-1 block text-sm text-danger">{said}</span>}
			</span>
			<div className="flex shrink-0 items-center gap-0.5">
				{onAskLoop !== undefined && (
					<button
						type="button"
						className="control btn-quiet btn-sm -mt-0.5"
						aria-label={`Ask about ${loop.summary}`}
						disabled={busy}
						onClick={() => onAskLoop(`About "${loop.summary}": `)}
					>
						Ask
					</button>
				)}
				<button
					type="button"
					className="control btn-icon -mr-1.5 -mt-1"
					aria-label={`Dismiss ${loop.summary}`}
					title="Dismiss"
					disabled={busy}
					onClick={() => void dismiss()}
				>
					<CloseIcon />
				</button>
			</div>
		</div>
	);
}
