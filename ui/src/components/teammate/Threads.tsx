import { useEffect, useState } from "react";
import type { PeerThreadSummary } from "../../generated/contract";
import { firstLine } from "../../room";
import { storedNumbers, storeNumbers } from "../../storage";
import { wire } from "../../wire";
import type { ThreadRef } from "../Transcript";
import { FoldRow, NESTED, RowText } from "./rows";

const THREAD_SEEN_KEY = "hotline.threads.seen";

/**
 * This teammate's conversations with other teammates, behind one row that
 * counts them; the right-hand pane opens one, and lists the rest.
 * Unread is lastAt against the latest the window has shown, the same way
 * the rail counts a tape.
 */
export function Threads({ personaId, onOpen }: { personaId: string; onOpen(thread: ThreadRef): void }) {
	const [threads, setThreads] = useState<PeerThreadSummary[] | undefined>(undefined);
	const [seen, setSeen] = useState(() => storedNumbers(THREAD_SEEN_KEY));
	const [open, setOpen] = useState(false);

	useEffect(() => {
		let cancelled = false;
		setOpen(false);
		void wire
			.command("peers.list", { personaId })
			.then((list) => {
				if (!cancelled) setThreads(list);
			})
			.catch(() => {
				if (!cancelled) setThreads([]);
			});
		return () => {
			cancelled = true;
		};
	}, [personaId]);

	useEffect(() => {
		if (threads === undefined) return;
		setSeen((current) => {
			let changed = false;
			const next = { ...current };
			for (const thread of threads) {
				if (next[thread.threadKey] === undefined) {
					next[thread.threadKey] = thread.lastAt;
					changed = true;
				}
			}
			if (!changed) return current;
			storeNumbers(THREAD_SEEN_KEY, next);
			return next;
		});
	}, [threads]);

	if (threads === undefined) return null;

	const isUnread = (thread: PeerThreadSummary) => thread.lastAt > (seen[thread.threadKey] ?? 0);
	const unread = threads.filter(isUnread).length;
	const total = threads.length;

	const openThread = (thread: PeerThreadSummary) => {
		setSeen((current) => {
			const next = { ...current, [thread.threadKey]: thread.lastAt };
			storeNumbers(THREAD_SEEN_KEY, next);
			return next;
		});
		onOpen({ thread: { kind: "pair", key: thread.threadKey }, withName: thread.withName });
	};

	const dot = <span aria-label="Unread" className="h-1.5 w-1.5 shrink-0 rounded-full bg-accent" />;

	return (
		<section>
			<h3 className="label">Threads</h3>
			<div className="grouped">
				{total === 0 ? (
					<div className="group-row">
						<RowText title="None yet" />
					</div>
				) : (
					<FoldRow
						title={total === 1 ? "1 thread" : `${total} threads`}
						{...(unread > 0 ? { value: `${unread} unread`, mark: dot } : {})}
						open={open}
						onToggle={() => setOpen((was) => !was)}
					/>
				)}
				{open &&
					threads.map((thread) => {
						const line = thread.preview
							? `${thread.preview.fromName}: ${firstLine(thread.preview.text)}`
							: thread.exchanges === 1
								? "1 exchange"
								: `${thread.exchanges} exchanges`;
						return (
							<button
								key={thread.threadKey}
								type="button"
								className={`${NESTED} group-row-choice w-full text-left`}
								onClick={() => openThread(thread)}
							>
								<span className="group-row-text">
									<span className="group-row-title">{thread.withName}</span>
									<span className="group-row-detail">{line}</span>
								</span>
								{isUnread(thread) && dot}
								<span className="shrink-0 text-xs text-ink-3">{threadStamp(thread.lastAt)}</span>
							</button>
						);
					})}
			</div>
		</section>
	);
}

const threadClock = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });
const threadDay = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric" });

function threadStamp(at: number): string {
	const when = new Date(at);
	const today = new Date();
	const sameDay =
		when.getFullYear() === today.getFullYear() &&
		when.getMonth() === today.getMonth() &&
		when.getDate() === today.getDate();
	return sameDay ? threadClock.format(when) : threadDay.format(when);
}
