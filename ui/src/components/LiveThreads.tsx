import { type KeyboardEvent, type Ref, useEffect, useState } from "react";
import { ChevronRightIcon } from "../icons";
import { type LiveThread, liveFor, threadsLabel, threadsWords } from "../liveThreads";
import { markOf } from "../threadList";

/**
 * The composer's count of the threads live in the session (task-437), at the
 * right of its bottom row in the person's accent, as the phone has it: "2
 * threads ›". It opens the panel of them over the field. Absent at zero.
 */
export function ThreadsKey({
	count,
	open,
	panelId,
	keyRef,
	onToggle,
	onKeyDown,
}: {
	count: number;
	open: boolean;
	panelId: string;
	keyRef: Ref<HTMLButtonElement>;
	/** `keyboard`: pressed with Enter or Space, so focus goes into the panel it opens. */
	onToggle(keyboard: boolean): void;
	onKeyDown(event: KeyboardEvent<HTMLButtonElement>): void;
}) {
	if (count <= 0) return null;
	return (
		<button
			ref={keyRef}
			type="button"
			className="composer-threads"
			aria-label={threadsLabel(count)}
			aria-expanded={open}
			aria-controls={open ? panelId : undefined}
			title={open ? "Hide threads" : "Show threads"}
			onClick={(event) => onToggle(event.detail === 0)}
			onKeyDown={onKeyDown}
		>
			<span>{threadsWords(count)}</span>
			<ChevronRightIcon className="composer-threads-chevron" />
		</button>
	);
}

/**
 * The live threads, in the pill's first row while the control is open: a
 * title saying how many, and a row each in the Threads list's shape, its
 * kind, what it is doing and how long it has run. A row opens its thread in
 * the right-hand pane, as the list's row does. Arrows, Home and End walk the
 * rows. Nothing else of the list is here: the full list stays in the pane.
 */
export function ThreadsPanel({
	id,
	threads,
	isOpen,
	onOpen,
	onEdge,
}: {
	id: string;
	threads: readonly LiveThread[];
	/** Whether a thread is the one open in the pane. */
	isOpen(thread: LiveThread): boolean;
	onOpen(thread: LiveThread): void;
	/** An arrow past the last row: back to the control below. */
	onEdge(): void;
}) {
	const now = useNow(1_000);
	const walk = (event: KeyboardEvent<HTMLElement>) => {
		const rows = [...event.currentTarget.querySelectorAll<HTMLElement>(".list-row")];
		const from = rows.indexOf(event.target as HTMLElement);
		if (from < 0) return;
		const to =
			event.key === "ArrowDown" ? from + 1 : event.key === "ArrowUp" ? Math.max(from - 1, 0) : event.key === "Home" ? 0 : event.key === "End" ? rows.length - 1 : undefined;
		if (to === undefined) return;
		event.preventDefault();
		if (to >= rows.length) onEdge();
		else rows[to]?.focus();
	};
	const label = threadsLabel(threads.length);
	return (
		<section id={id} className="composer-threads-panel" aria-label={label} onKeyDown={walk}>
			<h3 className="list-group">{label}</h3>
			<ul>
				{threads.map((thread) => {
					const mark = markOf(thread.kind);
					const time = liveFor(thread, now);
					return (
						<li key={thread.id}>
							<button
								type="button"
								className="list-row"
								data-thread={thread.id}
								data-kind={thread.kind}
								data-state={thread.working ? "live" : undefined}
								aria-current={isOpen(thread) || undefined}
								aria-label={`${thread.what}: ${thread.title}, ${thread.doing}, ${time}`}
								onClick={() => onOpen(thread)}
							>
								<span className="list-mark" aria-hidden="true">
									{mark.glyph}
								</span>
								<span className="min-w-0 flex-1">
									<span className="list-title">
										<span className="list-title-text">{thread.title}</span>
									</span>
									<span className="list-line">
										{thread.what} · {thread.doing}
									</span>
								</span>
								<span className="list-when">
									{time}
									{thread.working && <span aria-hidden="true" className="dock-dot beat" data-state="running" />}
								</span>
							</button>
						</li>
					);
				})}
			</ul>
		</section>
	);
}

/** The clock the rows' times read, ticking every `every` ms while the panel is open. */
function useNow(every: number): number {
	const [now, setNow] = useState(() => Date.now());
	useEffect(() => {
		const tick = window.setInterval(() => setNow(Date.now()), every);
		return () => window.clearInterval(tick);
	}, [every]);
	return now;
}
