/**
 * A failed turn's notice as the desk writes it (driver/failure.rs, `notice`):
 * the title and summary for an older client, then the envelope the window
 * reads. This one is the provider saying its model was at capacity.
 */
const providerReport = JSON.stringify(
	{
		sequence_number: 2,
		type: "error",
		error: { type: "service_unavailable_error", code: "server_is_overloaded", message: "The model is currently at capacity due to high demand. Please try again later." },
	},
	null,
	2,
);

export const CAPACITY_FAILURE = `Turn failed: The activity could not finish. Expand the details for the reported error.\n\n${JSON.stringify({
	hotlineFailure: {
		kind: "unknown",
		title: "Turn failed",
		summary: "The activity could not finish. Expand the details for the reported error.",
		details: providerReport,
		phase: "request",
		status: null,
		code: "null",
		retryAfterSeconds: null,
		toolsMayHaveRun: true,
	},
})}`;

/** What a failure drawn quietly says, and that none of its envelope is on show until Details is opened. */
export function quietFailure(root: ParentNode): { title: string | undefined; said: string | undefined; open: boolean; shown: string } {
	const line = root.querySelector<HTMLDetailsElement>("details.error-line");
	const summary = line?.querySelector("summary");
	return {
		title: line?.querySelector(".error-line-title")?.textContent ?? undefined,
		said: line?.querySelector(".error-line-said")?.textContent ?? undefined,
		open: line?.hasAttribute("open") ?? false,
		shown: summary?.textContent ?? "",
	};
}
