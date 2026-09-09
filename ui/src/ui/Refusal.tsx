import { useState } from "react";
import { ChevronDownIcon, ChevronRightIcon, WarningIcon } from "../icons";
import { openLink } from "../native";

/**
 * A refusal, wherever one lands: one sentence of ours beside the warning
 * glyph, and, when someone else had words — a daemon, a provider, a process
 * — those words behind a disclosure, selectable, with a page to read when
 * there is one. The sentence is the whole message; the rest is for whoever
 * wants it.
 */
export function Refusal({
	message,
	detail,
	docs,
}: {
	message: string;
	detail?: string;
	docs?: { title: string; href: string };
}) {
	const [open, setOpen] = useState(false);
	const more = detail !== undefined || docs !== undefined;
	return (
		<div role="status" className="refusal">
			<div className="refusal-row">
				<WarningIcon className="shrink-0 text-danger" />
				<span className="refusal-message selectable">{message}</span>
				{more && (
					<button
						type="button"
						className="control btn-quiet -mr-1 gap-1 px-2 text-sm"
						aria-expanded={open}
						onClick={() => setOpen((was) => !was)}
					>
						Details
						{open ? <ChevronDownIcon /> : <ChevronRightIcon />}
					</button>
				)}
			</div>
			{open && (
				<div className="refusal-more">
					{detail !== undefined && <pre className="refusal-detail selectable">{detail}</pre>}
					{docs !== undefined && (
						<button type="button" className="self-start text-sm underline" onClick={() => void openLink(docs.href)}>
							{docs.title}
						</button>
					)}
				</div>
			)}
		</div>
	);
}
