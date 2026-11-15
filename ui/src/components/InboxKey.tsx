import { type KeyboardEvent, type Ref, useEffect, useState } from "react";
import type { InboxItem } from "../generated/contract";
import { asideTime } from "../asides";
import { stopwatch } from "../dock";
import { ChevronRightIcon, InboxIcon } from "../icons";
import { senderWords, unreadWords } from "../inbox";

/**
 * The composer's count of what the teammate has not read in its inbox,
 * beside the threads' count and in its style: "2 unread". It opens the tray
 * of them over the field. Absent at zero.
 */
export function InboxKey({
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
	/** `keyboard`: pressed with Enter or Space, so focus goes into the tray it opens. */
	onToggle(keyboard: boolean): void;
	onKeyDown(event: KeyboardEvent<HTMLButtonElement>): void;
}) {
	if (count <= 0) return null;
	return (
		<button
			ref={keyRef}
			type="button"
			className="composer-threads composer-inbox"
			aria-label={`${count} unread in the inbox`}
			aria-expanded={open}
			aria-controls={open ? panelId : undefined}
			title={open ? "Hide unread" : "Show unread"}
			onClick={(event) => onToggle(event.detail === 0)}
			onKeyDown={onKeyDown}
		>
			<InboxIcon className="composer-inbox-mark" />
			<span>{unreadWords(count)}</span>
		</button>
	);
}

/**
 * What the teammate has not read, in the pill's first row while the count is
 * open: each item's title over its sender, and how long the sender has
 * waited, in amber, where it stopped for it (else when it was sent). A row
 * opens the item in the pane; the last opens the whole inbox. Arrows, Home
 * and End walk the rows.
 */
export function InboxPanel({
	id,
	items,
	nameOf,
	onOpen,
	onAll,
	onEdge,
}: {
	id: string;
	items: readonly InboxItem[];
	nameOf(personaId: string): string | undefined;
	onOpen(item: InboxItem): void;
	/** Opens the whole inbox in the pane. */
	onAll(): void;
	/** An arrow past the last row: back to the count below. */
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
	const label = unreadWords(items.length);
	return (
		<section id={id} className="composer-threads-panel" aria-label={label} onKeyDown={walk}>
			<h3 className="list-group">{label}</h3>
			<ul>
				{items.map((item) => {
					const waiting = item.waiting === true;
					const from = senderWords(item, nameOf);
					const when = waiting ? stopwatch(now - item.ts) : asideTime(item.ts, now);
					return (
						<li key={item.id}>
							<button
								type="button"
								className="list-row"
								data-inbox={item.id}
								aria-label={`${item.title}, from ${from}${waiting ? `, waiting ${when}` : `, ${when}`}`}
								onClick={() => onOpen(item)}
							>
								<span className="list-mark composer-inbox-ring" aria-hidden="true">
									○
								</span>
								<span className="min-w-0 flex-1">
									<span className="list-title">
										<span className="list-title-text">{item.title}</span>
									</span>
									<span className="list-line">{waiting ? `${from} · waiting` : from}</span>
								</span>
								<span className="list-when" data-waiting={waiting || undefined}>
									{when}
								</span>
							</button>
						</li>
					);
				})}
				<li>
					<button type="button" className="list-row composer-threads-all" onClick={onAll}>
						<span className="list-mark" aria-hidden="true">
							⋯
						</span>
						<span className="list-title min-w-0 flex-1">
							<span className="list-title-text">Whole inbox</span>
						</span>
						<span className="list-when">
							<ChevronRightIcon className="composer-threads-chevron" />
						</span>
					</button>
				</li>
			</ul>
		</section>
	);
}

/** The clock the rows' ages read, ticking every `every` ms while the tray is open. */
function useNow(every: number): number {
	const [now, setNow] = useState(() => Date.now());
	useEffect(() => {
		const tick = window.setInterval(() => setNow(Date.now()), every);
		return () => window.clearInterval(tick);
	}, [every]);
	return now;
}
