import { type ReactNode, useEffect, useLayoutEffect, useRef, useState } from "react";
import { ArrowDownIcon } from "../icons";

/** Slack under the newest line that still counts as following it. */
const FOLLOW_SLACK = 48;

/**
 * A window onto work as it happens: it follows the newest line until you
 * scroll back, and offers a way down again. The work card reads a turn
 * through it, and a thread's pane its live stream.
 */
export function FollowWindow({
	following: followKey,
	count,
	content,
	follow = true,
	frame: frameClass = "work-window",
	back = "Newest",
	live = false,
	children,
}: {
	/** What the window is following: a new one starts at its newest. */
	following: string | undefined;
	count: number;
	/** Changes when the visible body changes, including a growing thought or tool output. */
	content: unknown;
	/** Whether it starts at the newest; a page opened at a line it was asked for starts there instead. */
	follow?: boolean;
	/** The scrolling frame's class. */
	frame?: string;
	/** The word on the key that goes back down to the newest. */
	back?: string;
	/** The work is still going: the key breathes rather than points. */
	live?: boolean;
	children: ReactNode;
}) {
	const frame = useRef<HTMLDivElement>(null);
	const [following, setFollowing] = useState(follow);
	// Follow actual body changes, including text growing within one step.
	// Unrelated parent renders need no layout read.
	// biome-ignore lint/correctness/useExhaustiveDependencies: content and followKey identify the body whose layout changed.
	useLayoutEffect(() => {
		const el = frame.current;
		if (el && following) el.scrollTop = el.scrollHeight;
	}, [content, followKey, following]);
	// biome-ignore lint/correctness/useExhaustiveDependencies: a new followKey is new work to follow.
	useEffect(() => setFollowing(follow), [followKey]);
	// Opened at a line, with the whole run in view: there is nothing newer to go down to.
	// biome-ignore lint/correctness/useExhaustiveDependencies: measured once the body has laid out.
	useEffect(() => {
		const el = frame.current;
		if (el && !following && el.scrollHeight - el.scrollTop - el.clientHeight < FOLLOW_SLACK) setFollowing(true);
	}, [content, followKey]);

	return (
		<div className="relative flex min-h-0 flex-1 flex-col">
			<div
				ref={frame}
				className={frameClass}
				onScroll={(scroll) => {
					const el = scroll.currentTarget;
					const atEnd = el.scrollHeight - el.scrollTop - el.clientHeight < FOLLOW_SLACK;
					if (atEnd !== following) setFollowing(atEnd);
				}}
			>
				{children}
			</div>
			{!following && count > 0 && (
				<button
					type="button"
					className="control btn btn-sm work-newest gap-1"
					onClick={() => {
						setFollowing(true);
						frame.current?.scrollTo({ top: frame.current.scrollHeight, behavior: "smooth" });
					}}
				>
					{live ? <span aria-hidden="true" className="beat h-1.5 w-1.5 shrink-0 rounded-full bg-accent" /> : <ArrowDownIcon />}
					{back}
				</button>
			)}
		</div>
	);
}
