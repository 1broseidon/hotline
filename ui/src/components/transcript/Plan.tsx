import type { PlanEntry } from "../../generated/contract";
import { CheckIcon } from "../../icons";

/** A plan step's segment: done, the one being worked on, or not reached. Never part-filled. */
function planSegment(status: string): "done" | "now" | "todo" {
	return status === "completed" ? "done" : status === "in_progress" ? "now" : "todo";
}

/**
 * The agent's working list as a card that says how far along it is: a head
 * with "N of M done", one segment a step, full when it is done, lit for the
 * step under way and empty otherwise, then the steps themselves.
 */
export function Plan({ entries }: { entries: PlanEntry[] }) {
	if (entries.length === 0) return null;
	const done = entries.filter((entry) => entry.status === "completed").length;
	return (
		<section className="plan-card" aria-label={`Plan, ${done} of ${entries.length} done`}>
			<div className="plan-head">
				<span className="line-label">Plan</span>
				<span className="line-label">
					<b>{done}</b> of {entries.length} done
				</span>
			</div>
			<div className="plan-segments" aria-hidden="true">
				{entries.map((entry, index) => (
					<i key={`${index}:${entry.content}`} className={`plan-segment plan-segment-${planSegment(entry.status)}`} />
				))}
			</div>
			<ol className="plan-steps">
				{entries.map((entry, index) => {
					const segment = planSegment(entry.status);
					return (
						<li key={`${index}:${entry.content}`} className="plan-step" data-state={segment}>
							<span className={`plan-mark plan-mark-${segment}`} role="img" aria-label={segment === "done" ? "done" : segment === "now" ? "in progress" : entry.status}>
								{segment === "done" && <CheckIcon />}
							</span>
							<span className="min-w-0 flex-1">{entry.content}</span>
						</li>
					);
				})}
			</ol>
		</section>
	);
}
