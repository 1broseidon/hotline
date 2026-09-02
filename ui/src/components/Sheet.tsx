import { useEffect, type ReactNode } from "react";

/**
 * A sheet over the window: the app's one grammar for "make a thing" and for
 * "tell me something I have to say back". Escape is always the way out, so
 * every sheet has one door and it is the same door. A long sheet scrolls
 * inside the dialog rather than growing past the window.
 */
export function Sheet({
	title,
	onClose,
	children,
}: {
	title: string;
	onClose(): void;
	children: ReactNode;
}) {
	useEffect(() => {
		const close = (event: KeyboardEvent) => {
			if (event.key === "Escape") onClose();
		};
		window.addEventListener("keydown", close);
		return () => window.removeEventListener("keydown", close);
	}, [onClose]);

	return (
		<div
			className="absolute inset-0 z-10 grid place-items-center p-6"
			style={{ background: "oklch(6% 0.002 250 / 0.5)" }}
			onMouseDown={(event) => {
				if (event.target === event.currentTarget) onClose();
			}}
		>
			<div
				role="dialog"
				aria-modal="true"
				aria-label={title}
				className="max-h-full w-full max-w-md overflow-y-auto rounded-xl border border-rule bg-paper-2 p-5 shadow-2xl"
			>
				<h2 className="mb-4 text-base font-medium">{title}</h2>
				{children}
			</div>
		</div>
	);
}
