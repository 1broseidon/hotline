import type { ReactNode } from "react";

/**
 * A choice that has a good default: one line saying what it is now, opened
 * only to change it. New teammate is made of these, and so are the settings
 * panes, inside a `grouped` card or an `nt-folds` one.
 */
export function Fold({
	title,
	value,
	open,
	onToggle,
	action,
	children,
}: {
	title: string;
	value: ReactNode;
	open: boolean;
	onToggle(): void;
	/** The closed row's verb, when Change is not the right word ("Add key"). */
	action?: string;
	children: ReactNode;
}) {
	return (
		<div className="nt-fold" data-open={open ? "" : undefined}>
			<button type="button" className="nt-fold-row nt-fold-head" aria-expanded={open} onClick={onToggle}>
				<span className="nt-fold-title">{title}</span>
				<span className="nt-fold-value">{value}</span>
				<span className="nt-fold-action">{open ? "Done" : (action ?? "Change")}</span>
			</button>
			{open && <div className="nt-fold-body">{children}</div>}
		</div>
	);
}

/** One open fold at a time in a group, by name; opening the open one closes it. */
export function toggled<T extends string>(was: T | null, which: T): T | null {
	return was === which ? null : which;
}
