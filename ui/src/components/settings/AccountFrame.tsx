import { type ReactNode, useEffect, useRef } from "react";
import { initialOf } from "../../ui/Avatar";
import { BackKey, Band } from "../../ui/Band";
import { Scroll } from "../../ui/Scroll";

/** A page under Providers: its band with the way back, and its column. */
export function AccountFrame({ title, onBack, children }: { title: string; onBack(): void; children: ReactNode }) {
	return (
		<div className="pane">
			<Band>
				<BackKey onBack={onBack} />
				<h2 className="min-w-0 flex-1 truncate pl-1 text-lg font-semibold">{title}</h2>
			</Band>
			<Scroll>
				<div className="pane-column flex flex-col gap-6">{children}</div>
			</Scroll>
		</div>
	);
}

export function AccountCard({ name, title, detail, children }: { name: string; title?: ReactNode; detail: ReactNode; children?: ReactNode }) {
	return (
		<section className="nt-card">
			<div className="flex items-center gap-3">
				<span className="nt-avatar" aria-hidden="true">
					{initialOf(name)}
				</span>
				<span className="flex min-w-0 flex-1 flex-col gap-1">
					{title ?? <span className="welcome-card-title truncate">{name}</span>}
					<span className="flex min-w-0 items-center gap-2 text-sm text-ink-2">{detail}</span>
				</span>
			</div>
			{children}
		</section>
	);
}

/** Losing an account asks once, in place, before anything is sent. The button that asked is gone, so Cancel takes the focus. */
export function Confirm({
	question,
	detail,
	action,
	busy,
	onConfirm,
	onCancel,
}: {
	question: string;
	detail: string;
	action: string;
	busy: boolean;
	onConfirm(): void;
	onCancel(): void;
}) {
	const cancel = useRef<HTMLButtonElement>(null);
	useEffect(() => cancel.current?.focus(), []);
	return (
		// biome-ignore lint/a11y/noStaticElementInteractions: Escape is heard from the question's own buttons, as it bubbles.
		<div
			className="group-row flex-col items-stretch gap-2"
			onKeyDown={(event) => {
				if (event.key !== "Escape") return;
				// Escape here puts the question away; it does not also leave Settings.
				event.stopPropagation();
				onCancel();
			}}
		>
			<p>{question}</p>
			<p className="text-sm text-ink-3">{detail}</p>
			<div className="flex justify-end gap-2">
				<button ref={cancel} type="button" className="control btn-quiet" disabled={busy} onClick={onCancel}>
					Cancel
				</button>
				<button type="button" className="control btn btn-danger" disabled={busy} onClick={onConfirm}>
					{action}
				</button>
			</div>
		</div>
	);
}
